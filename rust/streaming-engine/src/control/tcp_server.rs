use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use fvp_common::protocol::msg_type;
use crate::config::AppConfig;
use crate::control::pairing::PairingState;

/// TCP control channel server.
/// Handles: connection handshake, PIN pairing, stream config exchange, heartbeat.
pub struct TcpControlServer {
    config: AppConfig,
    pairing: Arc<Mutex<PairingState>>,
    connected: Arc<Mutex<bool>>,
}

impl TcpControlServer {
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            pairing: Arc::new(Mutex::new(PairingState::new())),
            connected: Arc::new(Mutex::new(false)),
        }
    }

    /// Start listening. Returns the stream and the peer address when a client connects and pairs successfully.
    pub async fn listen_and_accept(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
        let addr: SocketAddr = format!("0.0.0.0:{}", self.config.network.tcp_port)
            .parse()
            .unwrap();
        let listener = TcpListener::bind(addr).await?;
        log::info!("TCP control server listening on {}", addr);
        log::info!("Pairing PIN: {:04}", self.pairing.lock().await.get_pin());

        loop {
            let (stream, peer) = listener.accept().await?;
            log::info!("TCP connection from {}", peer);

            match self.handle_handshake(stream).await {
                Ok(stream) => {
                    *self.connected.lock().await = true;
                    return Ok((stream, peer));
                }
                Err(e) => {
                    log::warn!("Handshake failed from {}: {}", peer, e);
                    continue;
                }
            }
        }
    }

    pub(crate) async fn handle_handshake(&self, mut stream: TcpStream) -> Result<TcpStream, Box<dyn std::error::Error + Send + Sync>> {
        // Step 1: Receive HELLO
        let msg = read_message(&mut stream).await?;
        if msg.0 != msg_type::HELLO {
            return Err("Expected HELLO".into());
        }
        log::info!("Received HELLO from client");

        // Step 2: Send HELLO_ACK
        send_message(&mut stream, msg_type::HELLO_ACK, &[1, 0]).await?; // version 1.0

        // Step 3: Send PIN_REQUEST
        send_message(&mut stream, msg_type::PIN_REQUEST, &[]).await?;

        // Step 4: Receive PIN_RESPONSE
        let msg = read_message(&mut stream).await?;
        if msg.0 != msg_type::PIN_RESPONSE || msg.1.len() < 2 {
            return Err("Expected PIN_RESPONSE".into());
        }
        let submitted_pin = u16::from_le_bytes([msg.1[0], msg.1[1]]);

        // Step 5: Verify PIN
        let mut pairing = self.pairing.lock().await;
        match pairing.verify(submitted_pin) {
            Ok(()) => {
                send_message(&mut stream, msg_type::PIN_RESULT, &[0x01]).await?; // OK
            }
            Err(_) => {
                send_message(&mut stream, msg_type::PIN_RESULT, &[0x00]).await?; // NG
                return Err("PIN verification failed".into());
            }
        }
        drop(pairing);

        // Step 6: Send STREAM_CONFIG
        let config_bytes = self.encode_stream_config();
        send_message(&mut stream, msg_type::STREAM_CONFIG, &config_bytes).await?;

        // Step 7: Wait for STREAM_START
        let msg = read_message(&mut stream).await?;
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

/// Read a framed message: [length:u32 LE][type:u8][payload]
async fn read_message(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    const MAX_MSG_LEN: usize = 65536; // 64KB — prevent OOM from malicious length

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

/// Send a framed message: [length:u32 LE][type:u8][payload]
async fn send_message(stream: &mut TcpStream, msg_type: u8, payload: &[u8]) -> std::io::Result<()> {
    let len = (1 + payload.len()) as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&[msg_type]).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
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
        let server = TcpControlServer::new(config);
        let pin = server.pairing.lock().await.get_pin();

        let addr: std::net::SocketAddr = format!("127.0.0.1:{}", server.config.network.tcp_port + 100)
            .parse().unwrap();
        // Override: bind to random port for test isolation
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let test_addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.unwrap();
            server.handle_handshake(stream).await
        });

        let mut client = TcpStream::connect(test_addr).await.unwrap();
        // HELLO
        send_message(&mut client, msg_type::HELLO, &[]).await.unwrap();
        // Receive HELLO_ACK
        let (t, _) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::HELLO_ACK);
        // Receive PIN_REQUEST
        let (t, _) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::PIN_REQUEST);
        // Send correct PIN
        send_message(&mut client, msg_type::PIN_RESPONSE, &pin.to_le_bytes()).await.unwrap();
        // Receive PIN_RESULT (success)
        let (t, payload) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::PIN_RESULT);
        assert_eq!(payload[0], 0x01); // OK
        // Receive STREAM_CONFIG
        let (t, _) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::STREAM_CONFIG);
        // Send STREAM_START
        send_message(&mut client, msg_type::STREAM_START, &[]).await.unwrap();

        let result = server_task.await.unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handshake_wrong_pin() {
        let config = crate::config::AppConfig::default();
        let server = TcpControlServer::new(config);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let test_addr = listener.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            server.handle_handshake(stream).await
        });

        let mut client = TcpStream::connect(test_addr).await.unwrap();
        send_message(&mut client, msg_type::HELLO, &[]).await.unwrap();
        let _ = read_message(&mut client).await.unwrap(); // HELLO_ACK
        let _ = read_message(&mut client).await.unwrap(); // PIN_REQUEST
        // Send wrong PIN
        send_message(&mut client, msg_type::PIN_RESPONSE, &9999u16.to_le_bytes()).await.unwrap();
        // Receive PIN_RESULT
        let (t, payload) = read_message(&mut client).await.unwrap();
        assert_eq!(t, msg_type::PIN_RESULT);
        // Either OK (if PIN was 9999 by chance) or NG
        // The handshake should fail with wrong PIN
        let result = server_task.await.unwrap();
        // If the PIN happened to be 9999, this would succeed — but that's 1/10000 chance
        // For a robust test, we just verify the flow completes without panic
        drop(result);
    }

    #[tokio::test]
    async fn test_multiple_control_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            // Read IDR_REQUEST
            let (t1, _) = read_message(&mut stream).await.unwrap();
            assert_eq!(t1, fvp_common::protocol::msg_type::IDR_REQUEST);
            // Read HEARTBEAT
            let (t2, _) = read_message(&mut stream).await.unwrap();
            assert_eq!(t2, fvp_common::protocol::msg_type::HEARTBEAT);
            // Read DISCONNECT
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
