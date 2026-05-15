//! Mock client and simulator harness internals.
//!
//! Used by both `bin/mock_client.rs` (CLI) and `tests/headless_e2e_test.rs`
//! (in-process). Gated behind the `simulator` feature so production driver
//! builds neither compile nor expose this code.
//!
//! The mock client speaks the same protocol as the production Android
//! receiver: TCP+TLS handshake (HELLO → PIN → STREAM_CONFIG → STREAM_START),
//! UDP receive of RTP packets, RtpDepacketizer reassembly, optional FEC
//! decode, periodic HEARTBEAT_ACK back over TCP. Synthetic tracking data
//! flows over UDP to the tracking port. Stats are aggregated and returned
//! when the run completes so tests can assert on them.

pub mod face_sender;
pub mod osc_loopback;
pub mod scenario;
pub mod test_helpers;
pub mod tracking_sender;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

use crate::transport::rtp::RtpDepacketizer;
use crate::transport::udp::UdpReceiver;

use self::face_sender::{encode_face_data, next_face_sample, FaceMode};
use self::osc_loopback::{OscCapture, OscLoopback};
use crate::engine::HapticEvent;

/// Configuration knobs for one mock-client run.
#[derive(Debug, Clone)]
pub struct MockClientConfig {
    /// `host:tcp_port` of the engine.
    pub server: SocketAddr,
    /// 6-digit PIN to submit (engine writes it to status.json on startup).
    pub pin: u32,
    /// How long to stream before disconnecting. `None` runs until cancelled.
    pub duration: Option<Duration>,
    /// UDP port to bind for incoming video. Convention: engine sends to
    /// `tcp_port + VIDEO_PORT_OFFSET = tcp_port + 2`. Default chooses that.
    pub video_udp_port: u16,
    /// UDP port to bind for incoming audio.
    pub audio_udp_port: u16,
    /// Tracking sender target (engine listens here).
    pub tracking_target: SocketAddr,
    /// HEARTBEAT_ACK cadence. Default 500 ms matches `HEARTBEAT_INTERVAL_MS`.
    pub heartbeat_interval: Duration,
    /// If `Some`, the stream loop emits FACE_DATA (0x35) at `face_send_interval`.
    pub face_pattern: Option<FaceMode>,
    /// How often to emit synthetic face data when `face_pattern` is set.
    pub face_send_interval: Duration,
    /// If `Some`, bind a UDP loopback on this port to capture OSC messages the
    /// engine's `OscBridge` emits. `MockClientStats::osc_messages` ends up with
    /// the captured `address → values` map.
    pub osc_listen_port: Option<u16>,
    /// When true, spawn a TCP reader task that decodes `HAPTIC_EVENT (0x38)`
    /// messages from the engine into `MockClientStats::haptic_events_received`.
    pub capture_haptic: bool,
}

impl MockClientConfig {
    /// Build with port conventions derived from the canonical UDP port:
    /// video = udp + 1, audio = udp + 3, tracking = udp + 2.
    pub fn from_ports(server_ip: std::net::IpAddr, tcp_port: u16, udp_port: u16, pin: u32) -> Self {
        Self {
            server: SocketAddr::new(server_ip, tcp_port),
            pin,
            duration: None,
            video_udp_port: udp_port + fvp_common::VIDEO_PORT_OFFSET,
            audio_udp_port: udp_port + fvp_common::AUDIO_PORT_OFFSET,
            tracking_target: SocketAddr::new(
                server_ip,
                udp_port + fvp_common::TRACKING_PORT_OFFSET,
            ),
            heartbeat_interval: Duration::from_millis(fvp_common::HEARTBEAT_INTERVAL_MS),
            face_pattern: None,
            face_send_interval: Duration::from_millis(50), // 20 Hz, plenty for OSC capture
            osc_listen_port: None,
            capture_haptic: false,
        }
    }
}

/// What a run measured. Tests assert on these.
#[derive(Debug, Default, Clone)]
pub struct MockClientStats {
    /// Frames fully reassembled by the depacketizer.
    pub frames_decoded: u64,
    /// IDR frames seen (subset of frames_decoded).
    pub idr_frames_seen: u64,
    /// RTP packets received on the video UDP socket.
    pub video_packets_received: u64,
    /// HEARTBEAT_ACK messages sent.
    pub heartbeats_sent: u64,
    /// FACE_DATA messages sent (mock-client → engine) when `face_pattern` is set.
    pub face_messages_sent: u64,
    /// Captured OSC traffic from the loopback receiver. Empty when
    /// `osc_listen_port` is `None`.
    pub osc_messages: OscCapture,
    /// HAPTIC_EVENT messages received from the engine (PC → HMD). Empty
    /// when `capture_haptic` is false.
    pub haptic_events_received: Vec<HapticEvent>,
    /// Connection establishment duration (TCP+TLS+handshake total).
    pub connect_duration: Duration,
    /// Total wall time the run spent streaming after the handshake.
    pub stream_duration: Duration,
}

/// Errors a mock-client run can surface to the caller.
#[derive(Debug)]
pub enum MockClientError {
    Io(std::io::Error),
    Tls(String),
    Protocol(String),
    PinRejected,
}

impl std::fmt::Display for MockClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O: {}", e),
            Self::Tls(s) => write!(f, "TLS: {}", s),
            Self::Protocol(s) => write!(f, "protocol: {}", s),
            Self::PinRejected => write!(f, "PIN rejected by server"),
        }
    }
}

impl std::error::Error for MockClientError {}

impl From<std::io::Error> for MockClientError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

/// Run the mock client end-to-end. Returns the aggregated stats on clean
/// exit. `cancel` is checked between events so callers (tests) can abort
/// without waiting for `duration`.
pub async fn run(
    config: MockClientConfig,
    cancel: CancellationToken,
) -> Result<MockClientStats, MockClientError> {
    // rustls 0.23 needs the process default crypto provider. Idempotent —
    // see the same call in StreamingEngine::new.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let connect_start = Instant::now();
    log::info!("mock-client connecting to {} (pin: {:06})", config.server, config.pin);

    let tcp = TcpStream::connect(config.server).await?;
    tcp.set_nodelay(true).ok();
    let mut stream = tls_handshake(tcp, &config.server.ip().to_string()).await?;
    do_handshake(&mut stream, config.pin).await?;
    let connect_duration = connect_start.elapsed();
    log::info!("mock-client handshake complete in {:?}", connect_duration);

    let stream_start = Instant::now();
    let stats = stream_loop(stream, &config, cancel.clone()).await?;
    let stats = MockClientStats {
        connect_duration,
        stream_duration: stream_start.elapsed(),
        ..stats
    };
    log::info!(
        "mock-client done: {} packets / {} frames ({} IDR) / {} HB-ACK in {:?}",
        stats.video_packets_received,
        stats.frames_decoded,
        stats.idr_frames_seen,
        stats.heartbeats_sent,
        stats.stream_duration,
    );
    Ok(stats)
}

/// Wrap the TCP stream in TLS. Accepts any server cert (TOFU semantics —
/// the production Android client pins the SHA-256 fingerprint at first
/// connect, but for the simulator we treat the engine's self-signed cert
/// as authoritative every time).
async fn tls_handshake(
    tcp: TcpStream,
    server_name: &str,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, MockClientError> {
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerifier))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    // Build a ServerName from the host string. Falls back to "localhost"
    // when the caller passes an IP literal (rustls 0.23 accepts both).
    let name = rustls::pki_types::ServerName::try_from(server_name.to_string())
        .or_else(|_| rustls::pki_types::ServerName::try_from("localhost".to_string()))
        .map_err(|e| MockClientError::Tls(format!("server name: {}", e)))?;
    connector
        .connect(name, tcp)
        .await
        .map_err(|e| MockClientError::Tls(format!("connector: {}", e)))
}

/// Cert verifier that accepts any server cert. Acceptable in the simulator
/// because we control both endpoints; production clients pin a fingerprint.
#[derive(Debug)]
struct SkipVerifier;

impl rustls::client::danger::ServerCertVerifier for SkipVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// HELLO → HELLO_ACK → PIN → PIN_RESULT → STREAM_CONFIG → STREAM_START.
/// Mirrors the server's `handle_handshake_generic` flow byte for byte.
async fn do_handshake<S>(stream: &mut S, pin: u32) -> Result<(), MockClientError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use fvp_common::protocol::msg_type;

    // HELLO with our protocol version.
    let hello_payload = fvp_common::protocol::encode_version(fvp_common::protocol::PROTOCOL_VERSION);
    send_message(stream, msg_type::HELLO, &hello_payload).await?;

    let (mt, _payload) = read_message(stream).await?;
    if mt != msg_type::HELLO_ACK {
        return Err(MockClientError::Protocol(
            format!("expected HELLO_ACK, got 0x{:02x}", mt),
        ));
    }

    // PIN_REQUEST from server, then PIN_RESPONSE with the 4-byte u32 PIN.
    let (mt, _) = read_message(stream).await?;
    if mt != msg_type::PIN_REQUEST {
        return Err(MockClientError::Protocol(
            format!("expected PIN_REQUEST, got 0x{:02x}", mt),
        ));
    }
    send_message(stream, msg_type::PIN_RESPONSE, &pin.to_le_bytes()).await?;

    let (mt, payload) = read_message(stream).await?;
    if mt != msg_type::PIN_RESULT {
        return Err(MockClientError::Protocol(
            format!("expected PIN_RESULT, got 0x{:02x}", mt),
        ));
    }
    if payload.first() != Some(&0x01) {
        return Err(MockClientError::PinRejected);
    }

    // STREAM_CONFIG (17-byte payload, parsed for log only — we don't enforce
    // the values match a local expectation, just acknowledge receipt).
    let (mt, payload) = read_message(stream).await?;
    if mt != msg_type::STREAM_CONFIG {
        return Err(MockClientError::Protocol(
            format!("expected STREAM_CONFIG, got 0x{:02x}", mt),
        ));
    }
    log::debug!("STREAM_CONFIG received ({} bytes)", payload.len());

    // STREAM_START closes the handshake.
    send_message(stream, msg_type::STREAM_START, &[]).await?;
    Ok(())
}

async fn send_message<S>(stream: &mut S, msg_type: u8, payload: &[u8]) -> Result<(), MockClientError>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let len = (1 + payload.len()) as u32;
    stream.write_all(&len.to_le_bytes()).await?;
    stream.write_all(&[msg_type]).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_message<S>(stream: &mut S) -> Result<(u8, Vec<u8>), MockClientError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > fvp_common::MAX_MSG_LEN {
        return Err(MockClientError::Protocol(
            format!("bad message length: {}", len),
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    let msg_type = buf[0];
    let payload = buf[1..].to_vec();
    Ok((msg_type, payload))
}

/// Post-handshake streaming loop. Concurrent tasks share a
/// `MockClientStats` via Arc<Mutex>: TCP read (HAPTIC capture), TCP
/// write (heartbeat + FACE_DATA), UDP receive + depacketize, OSC
/// loopback. Returns when any of them exits or `cancel` fires.
async fn stream_loop<S>(
    tcp: S,
    config: &MockClientConfig,
    cancel: CancellationToken,
) -> Result<MockClientStats, MockClientError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use fvp_common::protocol::msg_type;
    use std::sync::Mutex as StdMutex;

    let stats = Arc::new(StdMutex::new(MockClientStats::default()));

    // Split the TLS stream so a reader task can decode inbound TCP
    // messages (HAPTIC_EVENT etc.) while the main loop drives outbound
    // heartbeats and synthetic FACE_DATA. tokio's split keeps both
    // halves backed by the same underlying stream via an internal lock.
    let (mut tcp_read, mut tcp_write) = tokio::io::split(tcp);

    // Inbound TCP reader: capture HAPTIC_EVENT messages emitted by the
    // engine when production code (or scenario stimuli) call
    // `engine::queue_haptic`. Other inbound types are ignored — the
    // mock client doesn't act on them, and silently dropping keeps the
    // reader simple.
    let stats_reader = Arc::clone(&stats);
    let reader_cancel = cancel.clone();
    let capture_haptic = config.capture_haptic;
    let reader_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                r = read_message(&mut tcp_read) => match r {
                    Ok((mt, payload)) => {
                        if mt == msg_type::HAPTIC_EVENT && capture_haptic {
                            if let Some(event) = HapticEvent::from_payload(&payload) {
                                if let Ok(mut s) = stats_reader.lock() {
                                    s.haptic_events_received.push(event);
                                }
                            }
                        }
                        // All other inbound types intentionally ignored.
                    }
                    Err(e) => {
                        log::debug!("mock-client TCP reader exiting: {}", e);
                        break;
                    }
                },
                _ = reader_cancel.cancelled() => break,
            }
        }
    });

    // UDP receiver (video). Audio receiver omitted — the simulator's
    // current focus is video transport; audio plumbing lands in B6.
    let video_addr: SocketAddr = format!("0.0.0.0:{}", config.video_udp_port)
        .parse()
        .map_err(|e: std::net::AddrParseError| MockClientError::Protocol(e.to_string()))?;
    let receiver = UdpReceiver::new(video_addr).await?;
    log::info!("mock-client listening for video on {}", video_addr);

    let stats_video = Arc::clone(&stats);
    let video_cancel = cancel.clone();
    let video_handle = tokio::spawn(async move {
        let mut depacketizer = RtpDepacketizer::new();
        let mut buf = [0u8; 2048];
        while !video_cancel.is_cancelled() {
            tokio::select! {
                r = receiver.recv(&mut buf) => match r {
                    Ok((n, _peer)) => {
                        let mut s = stats_video.lock().unwrap();
                        s.video_packets_received += 1;
                        if let Some(frame) = depacketizer.feed(&buf[..n]) {
                            s.frames_decoded += 1;
                            if frame.is_keyframe {
                                s.idr_frames_seen += 1;
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("mock-client UDP recv skipped: {} ({:?})", e, e.kind());
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                },
                _ = video_cancel.cancelled() => break,
            }
        }
        log::info!("mock-client video receiver task exited");
    });

    // Optional OSC loopback: when set, bind a UDP receiver on the supplied
    // port and capture `/avatar/parameters/*` traffic emitted by the engine's
    // OscBridge. The handle survives the spawned task so we can snapshot
    // captured messages after the loop exits.
    let osc_state = if let Some(port) = config.osc_listen_port {
        match OscLoopback::bind(port).await {
            Ok(lb) => {
                log::debug!("mock-client: OSC loopback bound on port {}", lb.local_port());
                let handle = lb.capture_handle();
                let osc_cancel = cancel.clone();
                let task = tokio::spawn(async move {
                    lb.run(osc_cancel).await;
                });
                Some((handle, task))
            }
            Err(e) => {
                log::warn!("mock-client OSC loopback bind on port {} failed: {}", port, e);
                None
            }
        }
    } else {
        None
    };

    // Periodic HEARTBEAT_ACK on the TCP channel. Payload mirrors a minimal
    // production heartbeat (decode latency + packet loss percentage —
    // engine just parses for transport feedback).
    let deadline = config.duration.map(|d| Instant::now() + d);
    let heartbeat_interval = config.heartbeat_interval;
    let mut next_heartbeat = Instant::now() + heartbeat_interval;

    // Optional FACE_DATA emission on the same TCP channel. `next_face` is
    // `Some(deadline)` when `face_pattern` is set; we share the heartbeat
    // loop's timing logic rather than spinning up a second task because the
    // TCP writer can't be split cheaply without restructuring `send_message`.
    let face_send_interval = config.face_send_interval;
    let face_start = Instant::now();
    let mut next_face: Option<Instant> =
        config.face_pattern.map(|_| Instant::now() + face_send_interval);

    loop {
        if cancel.is_cancelled() {
            break;
        }
        if let Some(d) = deadline {
            if Instant::now() >= d {
                log::info!("mock-client duration reached, stopping");
                break;
            }
        }
        let now = Instant::now();

        // 6-byte payload: decode_latency_us:u32 (0) + packet_loss_pct:u16 (0)
        if now >= next_heartbeat {
            let payload = [0u8; 6];
            if let Err(e) = send_message(&mut tcp_write, msg_type::HEARTBEAT_ACK, &payload).await {
                log::warn!("mock-client heartbeat send failed: {}", e);
                break;
            }
            {
                let mut s = stats.lock().unwrap();
                s.heartbeats_sent += 1;
            }
            next_heartbeat = now + heartbeat_interval;
        }

        if let (Some(face_mode), Some(fire_at)) = (config.face_pattern, next_face) {
            if now >= fire_at {
                let t_ns = now.saturating_duration_since(face_start).as_nanos() as u64;
                let (lv, ev, lip, eye) = next_face_sample(face_mode, t_ns);
                let payload = encode_face_data(lv, ev, &lip, &eye);
                if let Err(e) = send_message(&mut tcp_write, msg_type::FACE_DATA, &payload).await {
                    log::warn!("mock-client face-data send failed: {}", e);
                    break;
                }
                {
                    let mut s = stats.lock().unwrap();
                    s.face_messages_sent += 1;
                }
                next_face = Some(now + face_send_interval);
            }
        }

        // Sleep until either the next heartbeat, next face send, or
        // cancel/deadline-check tick.
        let mut next_event = next_heartbeat;
        if let Some(f) = next_face {
            if f < next_event {
                next_event = f;
            }
        }
        if let Some(d) = deadline {
            if d < next_event {
                next_event = d;
            }
        }
        let now2 = Instant::now();
        let nap = if next_event > now2 {
            (next_event - now2).min(Duration::from_millis(100))
        } else {
            Duration::from_millis(10)
        };
        tokio::select! {
            _ = tokio::time::sleep(nap) => {}
            _ = cancel.cancelled() => break,
        }
    }

    // Polite DISCONNECT so the engine logs a clean shutdown instead of
    // counting this against `reconnect_attempts`.
    let _ = send_message(&mut tcp_write, msg_type::DISCONNECT, &[]).await;

    video_handle.abort();
    let _ = video_handle.await;

    // Brief drain window for haptic events still in flight from the engine,
    // then stop the reader. Without this delay, a stimulus fired right
    // before DISCONNECT would race the reader's exit and be missed.
    tokio::time::sleep(Duration::from_millis(50)).await;
    reader_handle.abort();
    let _ = reader_handle.await;

    // Drain OSC capture if we spawned it. The loopback's own cancel is fed
    // by the main `cancel` token; if the loop exited via `deadline` instead
    // of cancellation, abort the task so we don't dangle.
    let osc_messages = if let Some((handle, task)) = osc_state {
        // Give the receiver a brief moment to drain any in-flight UDP
        // packets before we tear it down. 50 ms is well under each
        // scenario's overhead budget.
        tokio::time::sleep(Duration::from_millis(50)).await;
        task.abort();
        let _ = task.await;
        handle.lock().map(|g| g.clone()).unwrap_or_default()
    } else {
        OscCapture::new()
    };

    let final_stats = {
        let mut s = stats.lock().unwrap();
        s.osc_messages = osc_messages;
        s.clone()
    };
    Ok(final_stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_ports_derives_offsets() {
        let cfg = MockClientConfig::from_ports(
            std::net::Ipv4Addr::LOCALHOST.into(),
            9944,
            9945,
            123456,
        );
        assert_eq!(cfg.server.port(), 9944);
        assert_eq!(cfg.video_udp_port, 9946); // udp + 1
        assert_eq!(cfg.audio_udp_port, 9948); // udp + 3
        assert_eq!(cfg.tracking_target.port(), 9947); // udp + 2
        assert_eq!(cfg.pin, 123456);
        assert_eq!(cfg.heartbeat_interval, Duration::from_millis(500));
    }

    #[test]
    fn test_stats_default_zero() {
        let s = MockClientStats::default();
        assert_eq!(s.frames_decoded, 0);
        assert_eq!(s.idr_frames_seen, 0);
        assert_eq!(s.video_packets_received, 0);
        assert_eq!(s.heartbeats_sent, 0);
    }
}
