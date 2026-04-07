use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;

use fvp_common::protocol::msg_type;
use crate::config::AppConfig;
use crate::control::pairing::PairingState;
use crate::control::tls;

/// Combined async read+write trait for boxed TLS or plain TCP streams.
/// Combined async read+write trait for boxed TLS or plain TCP streams.
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

/// TCP control channel server with TLS.
/// Handles: TLS handshake, connection handshake, PIN pairing, stream config, heartbeat.
pub struct TcpControlServer {
    config: AppConfig,
    pairing: Arc<Mutex<PairingState>>,
    connected: Arc<Mutex<bool>>,
    tls_acceptor: Option<TlsAcceptor>,
    cert_fingerprint: String,
}

impl TcpControlServer {
    pub fn new(config: AppConfig) -> Self {
        // Generate ephemeral TLS certificate
        let (tls_acceptor, cert_fingerprint) = match tls::create_tls_acceptor() {
            Ok((acceptor, fp)) => {
                log::info!("TLS enabled. Cert fingerprint: {}", fp);
                (Some(acceptor), fp)
            }
            Err(e) => {
                log::error!("TLS init failed: {}. Running without TLS!", e);
                (None, String::new())
            }
        };

        Self {
            config,
            pairing: Arc::new(Mutex::new(PairingState::new())),
            connected: Arc::new(Mutex::new(false)),
            tls_acceptor,
            cert_fingerprint,
        }
    }

    /// Create without TLS (for testing only).
    #[cfg(test)]
    pub(crate) fn new_without_tls(config: AppConfig) -> Self {
        Self {
            config,
            pairing: Arc::new(Mutex::new(PairingState::new())),
            connected: Arc::new(Mutex::new(false)),
            tls_acceptor: None,
            cert_fingerprint: String::new(),
        }
    }

    /// Get the TLS certificate fingerprint (SHA-256 hex) for TOFU pinning.
    pub fn cert_fingerprint(&self) -> &str {
        &self.cert_fingerprint
    }

    /// Start listening. Accepts TLS connection, then runs protocol handshake.
    /// Returns the authenticated stream (TLS-wrapped or plaintext) for post-handshake control.
    pub async fn listen_and_accept(&self) -> std::io::Result<(Box<dyn AsyncStream>, SocketAddr)> {
        let addr: SocketAddr = format!("0.0.0.0:{}", self.config.network.tcp_port)
            .parse()
            .unwrap();
        let listener = TcpListener::bind(addr).await?;
        log::info!("TCP control server listening on {} (TLS: {})",
            addr, self.tls_acceptor.is_some());
        log::info!("Pairing PIN: {:06}", self.pairing.lock().await.get_pin());

        loop {
            let (tcp_stream, peer) = listener.accept().await?;
            log::info!("TCP connection from {}", peer);

            if let Some(ref acceptor) = self.tls_acceptor {
                // TLS path
                match acceptor.accept(tcp_stream).await {
                    Ok(tls_stream) => {
                        match self.handle_handshake_generic(tls_stream).await {
                            Ok(stream) => {
                                *self.connected.lock().await = true;
                                log::info!("TLS handshake + pairing complete from {}", peer);
                                return Ok((Box::new(stream), peer));
                            }
                            Err(e) => {
                                log::warn!("Handshake failed from {}: {}", peer, e);
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("TLS handshake failed from {}: {}", peer, e);
                        continue;
                    }
                }
            } else {
                // Plaintext fallback (dev/test mode)
                match self.handle_handshake_generic(tcp_stream).await {
                    Ok(stream) => {
                        *self.connected.lock().await = true;
                        log::info!("Handshake + pairing complete from {}", peer);
                        return Ok((Box::new(stream), peer));
                    }
                    Err(e) => {
                        log::warn!("Handshake failed from {}: {}", peer, e);
                        continue;
                    }
                }
            }
        }
    }

    /// Generic handshake that works over any AsyncRead + AsyncWrite stream (TLS or plain TCP).
    pub(crate) async fn handle_handshake_generic<S>(&self, mut stream: S)
        -> Result<S, Box<dyn std::error::Error + Send + Sync>>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        // Step 1: Receive HELLO
        let msg = read_message_generic(&mut stream).await?;
        if msg.0 != msg_type::HELLO {
            return Err("Expected HELLO".into());
        }
        log::info!("Received HELLO from client");

        // Step 2: Send HELLO_ACK
        send_message_generic(&mut stream, msg_type::HELLO_ACK, &[1, 0]).await?;

        // Step 3: Send PIN_REQUEST
        send_message_generic(&mut stream, msg_type::PIN_REQUEST, &[]).await?;

        // Step 4: Receive PIN_RESPONSE
        let msg = read_message_generic(&mut stream).await?;
        if msg.0 != msg_type::PIN_RESPONSE || msg.1.len() < 4 {
            return Err("Expected PIN_RESPONSE (4 bytes)".into());
        }
        let submitted_pin = u32::from_le_bytes([msg.1[0], msg.1[1], msg.1[2], msg.1[3]]);

        // Step 5: Verify PIN
        let mut pairing = self.pairing.lock().await;
        match pairing.verify(submitted_pin) {
            Ok(()) => {
                send_message_generic(&mut stream, msg_type::PIN_RESULT, &[0x01]).await?;
            }
            Err(_) => {
                send_message_generic(&mut stream, msg_type::PIN_RESULT, &[0x00]).await?;
                return Err("PIN verification failed".into());
            }
        }
        drop(pairing);

        // Step 6: Send STREAM_CONFIG
        let config_bytes = self.encode_stream_config();
        send_message_generic(&mut stream, msg_type::STREAM_CONFIG, &config_bytes).await?;

        // Step 7: Wait for STREAM_START
        let msg = read_message_generic(&mut stream).await?;
        if msg.0 != msg_type::STREAM_START {
            return Err("Expected STREAM_START".into());
        }

        log::info!("Handshake complete, streaming ready");
        Ok(stream)
    }

    fn encode_stream_config(&self) -> Vec<u8> {
        let v = &self.config.video;
        let mut buf = Vec::new();
        buf.extend_from_slice(&v.resolution_per_eye[0].to_le_bytes());
        buf.extend_from_slice(&v.resolution_per_eye[1].to_le_bytes());
        buf.extend_from_slice(&v.bitrate_mbps.to_le_bytes());
        buf.extend_from_slice(&v.framerate.to_le_bytes());
        buf.push(match v.codec {
            fvp_common::protocol::VideoCodec::H264 => 0,
            fvp_common::protocol::VideoCodec::H265 => 1,
        });
        buf
    }

    pub fn is_connected(&self) -> Arc<Mutex<bool>> {
        self.connected.clone()
    }
}

/// Read a framed message from any async stream.
pub(crate) async fn read_message_generic<S>(stream: &mut S) -> std::io::Result<(u8, Vec<u8>)>
where
    S: AsyncRead + Unpin,
{
    const MAX_MSG_LEN: usize = 65536;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "empty message"));
    }
    if len > MAX_MSG_LEN {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,
            format!("message too large: {} bytes", len)));
    }

    let mut msg_buf = vec![0u8; len];
    stream.read_exact(&mut msg_buf).await?;

    let msg_type = msg_buf[0];
    let payload = msg_buf[1..].to_vec();
    Ok((msg_type, payload))
}

/// Send a framed message to any async stream.
pub(crate) async fn send_message_generic<S>(stream: &mut S, msg_type: u8, payload: &[u8]) -> std::io::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let len = (1 + payload.len()) as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&[msg_type]).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

// Convenience aliases for tests that use TcpStream directly
#[cfg(test)]
use tokio::net::TcpStream;

#[cfg(test)]
async fn read_message(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    read_message_generic(stream).await
}

#[cfg(test)]
async fn send_message(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> std::io::Result<()> {
    send_message_generic(stream, msg_type, payload).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_message_framing_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (msg_type, payload) = read_message(&mut stream).await.unwrap();
            assert_eq!(msg_type, 0x42);
            assert_eq!(payload, vec![1, 2, 3]);
            send_message(&mut stream, 0x43, &[4, 5]).await.unwrap();
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        send_message(&mut client, 0x42, &[1, 2, 3]).await.unwrap();
        let (msg_type, payload) = read_message(&mut client).await.unwrap();
        assert_eq!(msg_type, 0x43);
        assert_eq!(payload, vec![4, 5]);

        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_idr_request_message() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (msg_type, payload) = read_message(&mut stream).await.unwrap();
            assert_eq!(msg_type, fvp_common::protocol::msg_type::IDR_REQUEST);
            assert!(payload.is_empty());
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        send_message(&mut client, fvp_common::protocol::msg_type::IDR_REQUEST, &[]).await.unwrap();

        server.await.unwrap();
    }

    #[tokio::test]
    async fn test_full_handshake_success() {
        let config = crate::config::AppConfig::default();
        let server = TcpControlServer::new_without_tls(config);
        let pin = server.pairing.lock().await.get_pin();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let test_addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            server.handle_handshake_generic(stream).await
        });

        let mut client = TcpStream::connect(test_addr).await.unwrap();
        send_message(&mut client, msg_type::HELLO, &[]).await.unwrap();
        let (t, _) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::HELLO_ACK);
        let (t, _) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::PIN_REQUEST);
        send_message(&mut client, msg_type::PIN_RESPONSE, &pin.to_le_bytes()).await.unwrap();
        let (t, payload) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::PIN_RESULT);
        assert_eq!(payload[0], 0x01);
        let (t, _) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::STREAM_CONFIG);
        send_message(&mut client, msg_type::STREAM_START, &[]).await.unwrap();

        let result = server_task.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handshake_wrong_pin() {
        let config = crate::config::AppConfig::default();
        let server = TcpControlServer::new_without_tls(config);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let test_addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            server.handle_handshake_generic(stream).await
        });

        let mut client = TcpStream::connect(test_addr).await.unwrap();
        send_message(&mut client, msg_type::HELLO, &[]).await.unwrap();
        let _ = read_message(&mut client).await.unwrap();
        let _ = read_message(&mut client).await.unwrap();
        send_message(&mut client, msg_type::PIN_RESPONSE, &999999u32.to_le_bytes()).await.unwrap();
        let (t, _payload) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::PIN_RESULT);
        let result = server_task.await.unwrap();
        drop(result);
    }

    #[tokio::test]
    async fn test_multiple_control_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (t1, _) = read_message(&mut stream).await.unwrap();
            assert_eq!(t1, fvp_common::protocol::msg_type::IDR_REQUEST);
            let (t2, _) = read_message(&mut stream).await.unwrap();
            assert_eq!(t2, fvp_common::protocol::msg_type::HEARTBEAT);
            let (t3, _) = read_message(&mut stream).await.unwrap();
            assert_eq!(t3, fvp_common::protocol::msg_type::DISCONNECT);
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        send_message(&mut client, fvp_common::protocol::msg_type::IDR_REQUEST, &[]).await.unwrap();
        send_message(&mut client, fvp_common::protocol::msg_type::HEARTBEAT, &[]).await.unwrap();
        send_message(&mut client, fvp_common::protocol::msg_type::DISCONNECT, &[]).await.unwrap();

        server.await.unwrap();
    }
}
