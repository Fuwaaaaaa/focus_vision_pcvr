use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::control::tcp_server::TcpControlServer;
use crate::metrics::latency::{FrameTimestamps, LatencyTracker};
use crate::pipeline;
use crate::tracking::receiver::TrackingReceiver;
use crate::transport::rtp::RtpPacketizer;
use crate::transport::udp::UdpSender;
use fvp_common::protocol::{ControllerState, TrackingData};

/// Callback type for IDR request notifications.
/// Set via fvp_set_idr_callback() from C++.
static IDR_CALLBACK: std::sync::RwLock<Option<extern "C" fn()>> = std::sync::RwLock::new(None);

/// Callback for gaze updates — forwards eye tracking data to NvencEncoder.
/// Set via fvp_set_gaze_callback() from C++.
static GAZE_CALLBACK: std::sync::RwLock<Option<extern "C" fn(f32, f32, i32)>> = std::sync::RwLock::new(None);

/// Callback for bitrate changes — tells C++ NvencEncoder to adjust bitrate.
/// Set via fvp_set_bitrate_callback() from C++.
static BITRATE_CALLBACK: std::sync::RwLock<Option<extern "C" fn(u32)>> = std::sync::RwLock::new(None);

pub fn set_idr_callback(cb: extern "C" fn()) {
    if let Ok(mut guard) = IDR_CALLBACK.write() {
        *guard = Some(cb);
    }
}

fn notify_idr_request() {
    if let Ok(guard) = IDR_CALLBACK.read() {
        if let Some(cb) = *guard {
            cb();
        }
    }
}

pub fn set_gaze_callback(cb: extern "C" fn(f32, f32, i32)) {
    if let Ok(mut guard) = GAZE_CALLBACK.write() {
        *guard = Some(cb);
    }
}

pub fn set_bitrate_callback(cb: extern "C" fn(u32)) {
    if let Ok(mut guard) = BITRATE_CALLBACK.write() {
        *guard = Some(cb);
    }
}

fn notify_bitrate_change(bitrate_bps: u32) {
    if let Ok(guard) = BITRATE_CALLBACK.read() {
        if let Some(cb) = *guard {
            cb(bitrate_bps);
        }
    }
}

pub fn notify_gaze_update(x: f32, y: f32, valid: bool) {
    if let Ok(guard) = GAZE_CALLBACK.read() {
        if let Some(cb) = *guard {
            cb(x, y, if valid { 1 } else { 0 });
        }
    }
}

/// H.265 encoded frame data submitted from the C++ OpenVR driver.
/// The C++ driver handles D3D11 texture capture, NV12 conversion, and
/// NVENC encoding. Rust receives only the encoded NAL units.
pub struct EncodedFrame {
    pub frame_index: u32,
    pub nal_data: Vec<u8>,
    pub is_idr: bool,
    pub timestamps: FrameTimestamps,
}

/// The main streaming engine running on a tokio runtime.
pub struct StreamingEngine {
    #[allow(dead_code)] // Kept alive to prevent tokio runtime drop
    runtime: Runtime,
    frame_tx: mpsc::Sender<EncodedFrame>,
    latest_tracking: Arc<StdMutex<Option<TrackingData>>>,
    latest_controllers: Arc<StdMutex<[Option<ControllerState>; 2]>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
    cancel_token: CancellationToken,
    #[allow(dead_code)] // Available for future config queries
    config: AppConfig,
}

impl StreamingEngine {
    pub fn new(config: AppConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Build tokio runtime with limited threads (eng review decision #1)
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("fvp-stream")
            .build()?;

        let (frame_tx, frame_rx) = mpsc::channel::<EncodedFrame>(4);
        let latest_tracking = Arc::new(StdMutex::new(None));
        let latest_controllers: Arc<StdMutex<[Option<ControllerState>; 2]>> =
            Arc::new(StdMutex::new([None, None]));
        let latency_tracker = Arc::new(StdMutex::new(LatencyTracker::new(90)));

        let cancel_token = CancellationToken::new();
        let tracking_clone = latest_tracking.clone();
        let tracker_clone = latency_tracker.clone();
        let config_clone = config.clone();

        // Spawn the main streaming task
        let cancel = cancel_token.clone();
        let stream_cancel = cancel_token.clone();
        runtime.spawn(async move {
            tokio::select! {
                result = run_streaming(config_clone, frame_rx, tracking_clone, tracker_clone, stream_cancel) => {
                    if let Err(e) = result {
                        log::error!("Streaming engine error: {}", e);
                    }
                }
                _ = cancel.cancelled() => {
                    log::info!("Streaming task cancelled");
                }
            }
        });

        // Spawn tracking receiver (UDP, separate port)
        let tracking_head = latest_tracking.clone();
        let tracking_ctrl = latest_controllers.clone();
        let tracking_port = config.network.udp_port + fvp_common::TRACKING_PORT_OFFSET;
        let cancel = cancel_token.clone();
        runtime.spawn(async move {
            let receiver = TrackingReceiver::new(tracking_head, tracking_ctrl);
            let addr: SocketAddr = match format!("0.0.0.0:{}", tracking_port).parse() {
                Ok(a) => a,
                Err(e) => {
                    log::error!("Invalid tracking port {}: {}", tracking_port, e);
                    return;
                }
            };
            tokio::select! {
                result = receiver.run(addr) => {
                    if let Err(e) = result {
                        log::error!("Tracking receiver error: {}", e);
                    }
                }
                _ = cancel.cancelled() => {
                    log::info!("Tracking receiver cancelled");
                }
            }
        });

        Ok(Self {
            runtime,
            frame_tx,
            latest_tracking,
            latest_controllers,
            latency_tracker,
            cancel_token,
            config,
        })
    }

    /// Submit a frame for encoding and sending. Called from C++ thread.
    pub fn submit_frame(&self, frame: EncodedFrame) -> bool {
        match self.frame_tx.try_send(frame) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::warn!("Frame dropped: send channel full");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::error!("Frame channel closed");
                false
            }
        }
    }

    /// Get latest tracking data. Called from C++ thread.
    pub fn get_tracking(&self) -> Option<TrackingData> {
        *self.latest_tracking.lock().map_err(|e| log::error!("Tracking lock poisoned: {}", e)).ok()?
    }

    /// Get latest controller state. Called from C++ thread.
    /// `id`: 0 = left, 1 = right.
    pub fn get_controller(&self, id: u8) -> Option<ControllerState> {
        let guard = self.latest_controllers.lock().map_err(|e| log::error!("Controller lock poisoned: {}", e)).ok()?;
        let idx = id as usize;
        if idx < 2 { guard[idx] } else { None }
    }

    /// Cancel all async tasks for graceful shutdown.
    pub fn shutdown(&self) {
        self.cancel_token.cancel();
    }

    /// Log latency stats periodically.
    pub fn log_stats(&self) {
        if let Ok(tracker) = self.latency_tracker.lock() {
            if let Some(avg) = tracker.avg_pc_latency_us() {
                log::info!(
                    "Latency (PC side): avg={}us, encode={}us, frames={}",
                    avg,
                    tracker.avg_encode_latency_us().unwrap_or(0),
                    tracker.frame_count()
                );
            }
        }
    }
}

/// Heartbeat stats received from HMD.
/// Parsed from HEARTBEAT TCP message payload.
pub struct HmdStats {
    pub packets_received: u32,
    pub packets_lost: u32,
    pub avg_decode_us: u32,
    pub fps: u16,
}

/// Read TCP control messages after handshake (IDR_REQUEST, HEARTBEAT, FACE_DATA, DISCONNECT).
/// When the connection closes or errors, cancels the provided token to stop streaming.
/// Accepts any async stream (TLS-wrapped or plaintext) from the handshake.
async fn handle_tcp_control(
    mut stream: Box<dyn crate::control::tcp_server::AsyncStream>,
    cancel: tokio_util::sync::CancellationToken,
    hmd_stats: Arc<StdMutex<Option<HmdStats>>>,
    osc_bridge: Arc<StdMutex<crate::face_tracking::osc_bridge::OscBridge>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::AsyncReadExt;

    const MAX_MSG_LEN: usize = 65536; // 64KB — control messages are small

    loop {
        // Read framed message: [length:u32 LE][type:u8][payload]
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            log::info!("TCP control connection closed — stopping stream");
            cancel.cancel();
            break;
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        if len == 0 { continue; }
        if len > MAX_MSG_LEN {
            log::error!("TCP message too large ({} bytes), closing connection", len);
            cancel.cancel();
            break;
        }

        let mut msg_buf = vec![0u8; len];
        if stream.read_exact(&mut msg_buf).await.is_err() {
            log::info!("TCP control read failed mid-message — stopping stream");
            cancel.cancel();
            break;
        }
        let msg_type = msg_buf[0];

        match msg_type {
            fvp_common::protocol::msg_type::IDR_REQUEST => {
                log::info!("Received IDR_REQUEST from client");
                notify_idr_request();
            }
            fvp_common::protocol::msg_type::HEARTBEAT => {
                // Parse heartbeat payload: [seq:4][timestamp:8][stats:14]
                let payload = &msg_buf[1..];
                if payload.len() >= 26 {
                    let stats_offset = 12; // skip seq(4) + timestamp(8)
                    let s = &payload[stats_offset..];
                    let packets_received = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    let packets_lost = u32::from_le_bytes([s[4], s[5], s[6], s[7]]);
                    let avg_decode_us = u32::from_le_bytes([s[8], s[9], s[10], s[11]]);
                    let fps = u16::from_le_bytes([s[12], s[13]]);

                    if let Ok(mut guard) = hmd_stats.lock() {
                        *guard = Some(HmdStats {
                            packets_received,
                            packets_lost,
                            avg_decode_us,
                            fps,
                        });
                    }
                }
            }
            fvp_common::protocol::msg_type::FACE_DATA => {
                let payload = &msg_buf[1..];
                if let Some((lip_valid, eye_valid, lip, eye)) =
                    crate::face_tracking::osc_bridge::parse_face_data(payload)
                {
                    if let Ok(mut bridge) = osc_bridge.lock() {
                        bridge.send_face_data(lip_valid, eye_valid, &lip, &eye);
                    }
                }
            }
            fvp_common::protocol::msg_type::DISCONNECT => {
                log::info!("Client sent DISCONNECT — stopping stream");
                cancel.cancel();
                break;
            }
            _ => {
                log::debug!("Unknown TCP control message type: 0x{:02x}", msg_type);
            }
        }
    }
    Ok(())
}

/// Spawn the audio capture → Opus encode → UDP send pipeline.
/// Audio is optional: if capture or encoding fails, streaming continues without audio.
fn spawn_audio_pipeline(target: SocketAddr, cancel: CancellationToken) {
    use crate::audio::{capture::AudioCapture, encoder::AudioEncoder};
    use crate::transport::rtp::RtpPacket;

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<f32>>(32);

    // Create and hold AudioCapture on a dedicated thread.
    // cpal Stream is !Send, so it must live on the thread where it was created.
    // The thread blocks until the cancel token fires, then drops the capture.
    // Poll is_cancelled() every 100ms — avoids creating a tokio runtime just to wait.
    let hold_cancel = cancel.clone();
    std::thread::Builder::new()
        .name("fvp-audio-capture".into())
        .spawn(move || {
            let _capture = match AudioCapture::start(audio_tx) {
                Some(c) => c,
                None => {
                    log::info!("Audio capture unavailable — streaming video only");
                    return;
                }
            };
            while !hold_cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            log::info!("Audio capture released");
        })
        .expect("spawn audio capture thread");

    // Spawn async task: accumulate raw chunks into 10ms frames, encode, send.
    // Accumulation happens here (not in the real-time callback) to avoid Mutex.
    const OPUS_FRAME_SAMPLES: usize = 480; // 10ms at 48kHz
    const STEREO_FRAME_SIZE: usize = OPUS_FRAME_SAMPLES * 2;

    tokio::spawn(async move {
        let mut encoder = match AudioEncoder::new(128_000) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Opus encoder init failed: {} — no audio", e);
                return;
            }
        };

        let udp_sender = match UdpSender::new(target).await {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Audio UDP sender failed: {} — no audio", e);
                return;
            }
        };

        let mut sequence: u16 = 0;
        let mut timestamp: u32 = 0;
        let ssrc: u32 = 0x41554449; // "AUDI"
        let mut accum: Vec<f32> = Vec::with_capacity(STEREO_FRAME_SIZE * 2);

        log::info!("Audio streaming started to {}", target);

        loop {
            tokio::select! {
                Some(chunk) = audio_rx.recv() => {
                    // Accumulate raw samples from capture callback
                    accum.extend_from_slice(&chunk);

                    // Extract and encode complete 10ms frames
                    while accum.len() >= STEREO_FRAME_SIZE {
                        let pcm_frame: Vec<f32> = accum.drain(..STEREO_FRAME_SIZE).collect();

                        let opus_data = match encoder.encode(&pcm_frame) {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Opus encode error: {}", e);
                                continue;
                            }
                        };

                        // Build RTP packet: header (12 bytes) + Opus payload
                        let mut buf = Vec::with_capacity(12 + opus_data.len());
                        buf.push(0x80); // V=2, P=0, X=0, CC=0
                        buf.push(0x80 | 111); // M=1 (always for audio), PT=111 (Opus)
                        buf.extend_from_slice(&sequence.to_be_bytes());
                        buf.extend_from_slice(&timestamp.to_be_bytes());
                        buf.extend_from_slice(&ssrc.to_be_bytes());
                        buf.extend_from_slice(&opus_data);

                        let packet = RtpPacket { data: buf };
                        if let Err(e) = udp_sender.send_all(&[packet]).await {
                            log::debug!("Audio UDP send error: {}", e);
                        }

                        sequence = sequence.wrapping_add(1);
                        timestamp = timestamp.wrapping_add(480);
                    }
                }
                _ = cancel.cancelled() => {
                    log::info!("Audio streaming cancelled");
                    break;
                }
            }
        }
    });

}

async fn run_streaming(
    config: AppConfig,
    mut frame_rx: mpsc::Receiver<EncodedFrame>,
    _tracking: Arc<StdMutex<Option<TrackingData>>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Reconnect loop: when a session ends (TCP disconnect, Wi-Fi drop),
    // clean up and re-listen for a new HMD connection.
    let mut attempt: u32 = 0;
    const MAX_RECONNECT_ATTEMPTS: u32 = 5;
    let backoff_base = std::time::Duration::from_secs(1);

    loop {
        if cancel.is_cancelled() { break; }

        if attempt > 0 {
            let delay = backoff_base * 2u32.pow((attempt - 1).min(4));
            log::info!("Reconnecting in {:?} (attempt {}/{})", delay, attempt, MAX_RECONNECT_ATTEMPTS);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = cancel.cancelled() => { break; }
            }
        }

        // Step 1: Wait for HMD to connect via TCP
        let tcp_server = TcpControlServer::new(config.clone());
        let accept_result = tokio::select! {
            r = tcp_server.listen_and_accept() => r,
            _ = cancel.cancelled() => { break; }
        };

        let (tcp_control_stream, peer_addr) = match accept_result {
            Ok(r) => r,
            Err(e) => {
                log::error!("TCP accept failed: {}", e);
                attempt += 1;
                if attempt > MAX_RECONNECT_ATTEMPTS {
                    log::error!("Max reconnect attempts reached, stopping");
                    break;
                }
                continue;
            }
        };

        log::info!("HMD connected from {}, starting video stream", peer_addr);
        attempt = 0; // Reset on successful connection

        // Per-session cancel: fires when TCP drops or HMD disconnects
        let session_cancel = CancellationToken::new();

        // Shared HMD stats for adaptive bitrate (fed by heartbeat messages)
        let hmd_stats: Arc<StdMutex<Option<HmdStats>>> = Arc::new(StdMutex::new(None));

        // OSC bridge for face tracking data (HMD → VRChat)
        let osc_bridge = Arc::new(StdMutex::new(
            crate::face_tracking::osc_bridge::OscBridge::with_smoothing(config.face_tracking.smoothing)
        ));

        // Spawn TCP control reader (uses the same TLS/plain stream from handshake)
        let tcp_session = session_cancel.clone();
        let stats_clone = hmd_stats.clone();
        let osc_clone = osc_bridge.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_tcp_control(tcp_control_stream, tcp_session, stats_clone, osc_clone).await {
                log::warn!("TCP control reader ended: {}", e);
            }
        });

        if config.foveated.enabled {
            log::info!(
                "Foveated encoding enabled: fovea={:.0}%, mid={:.0}%, QP+{}/+{}",
                config.foveated.fovea_radius * 100.0,
                config.foveated.mid_radius * 100.0,
                config.foveated.mid_qp_offset,
                config.foveated.peripheral_qp_offset,
            );
        }

        // Step 2: Create UDP senders
        let udp_target: SocketAddr = SocketAddr::new(peer_addr.ip(), config.network.udp_port + fvp_common::VIDEO_PORT_OFFSET);
        let udp_sender = match UdpSender::new(udp_target).await {
            Ok(s) => s,
            Err(e) => {
                log::error!("UDP sender failed: {}", e);
                session_cancel.cancel();
                attempt += 1;
                continue;
            }
        };

        // Audio pipeline (per-session)
        let audio_port = config.network.udp_port + fvp_common::AUDIO_PORT_OFFSET;
        let audio_target: SocketAddr = SocketAddr::new(peer_addr.ip(), audio_port);
        spawn_audio_pipeline(audio_target, session_cancel.clone());

        // Step 3: Process frames with adaptive bitrate (HMD-reported loss)
        let mut packetizer = RtpPacketizer::new(0x46565000);
        let mut fec_encoder = crate::transport::fec::FecEncoder::new(config.network.fec_redundancy);
        let mut frame_count: u64 = 0;

        let mut bw_estimator = crate::adaptive::bandwidth_estimator::BandwidthEstimator::new();
        let mut bitrate_ctrl = crate::adaptive::bitrate_controller::BitrateController::new(
            config.video.bitrate_mbps,
        );

        loop {
            tokio::select! {
                frame_opt = frame_rx.recv() => {
                    let mut frame = match frame_opt {
                        Some(f) => f,
                        None => break, // Channel closed (engine shutdown)
                    };

                    frame.timestamps.mark_encode_start();
                    frame.timestamps.mark_encode_end();

                    let timestamp_90khz = (frame_count * (fvp_common::RTP_CLOCK_RATE as u64 / 90)) as u32;

                    let packets = pipeline::encode_frame_to_packets_with_fec(
                        &frame.nal_data,
                        frame.frame_index,
                        timestamp_90khz,
                        frame.is_idr,
                        &mut fec_encoder,
                        &mut packetizer,
                    );

                    if let Err(e) = udp_sender.send_all(&packets).await {
                        log::warn!("UDP send error: {}", e);
                    }

                    // Return packet buffers to the pool for reuse on the next frame
                    packetizer.recycle(packets);

                    frame.timestamps.mark_send();

                    if let Ok(mut tracker) = latency_tracker.lock() {
                        tracker.record(frame.timestamps);
                    }

                    frame_count += 1;

                    // Adaptive bitrate: use HMD-reported stats (real packet loss)
                    if frame_count.is_multiple_of(90) {
                        if let Ok(mut guard) = hmd_stats.lock() {
                            if let Some(stats) = guard.take() {
                                bw_estimator.update(
                                    stats.packets_received,
                                    stats.packets_lost,
                                    0.0, // RTT not yet measured
                                );
                                if bitrate_ctrl.adjust(&bw_estimator) {
                                    let new_bps = bitrate_ctrl.current_bitrate_bps() as u32;
                                    notify_bitrate_change(new_bps);
                                }
                            }
                        }
                    }

                    // Log stats every 5 seconds
                    if frame_count.is_multiple_of(450) {
                        if let Ok(tracker) = latency_tracker.lock() {
                            if let Some(avg) = tracker.avg_pc_latency_us() {
                                log::info!("PC latency: avg={}us encode={}us",
                                    avg, tracker.avg_encode_latency_us().unwrap_or(0));
                            }
                        }
                    }
                }
                _ = session_cancel.cancelled() => {
                    log::info!("Session ended — waiting for new connection");
                    break;
                }
                _ = cancel.cancelled() => {
                    log::info!("Engine shutdown — stopping streaming");
                    return Ok(());
                }
            }
        }

        // Session ended — loop back to accept new connection
        attempt += 1;
        if attempt > MAX_RECONNECT_ATTEMPTS {
            log::error!("Max reconnect attempts reached");
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::metrics::latency::FrameTimestamps;

    #[test]
    fn test_engine_creation() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config);
        assert!(engine.is_ok());
        let engine = engine.unwrap();
        engine.shutdown();
    }

    #[test]
    fn test_get_tracking_none_initially() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        assert!(engine.get_tracking().is_none());
        engine.shutdown();
    }

    #[test]
    fn test_get_controller_none_initially() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        assert!(engine.get_controller(0).is_none());
        assert!(engine.get_controller(1).is_none());
        engine.shutdown();
    }

    #[test]
    fn test_get_controller_invalid_id() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        assert!(engine.get_controller(2).is_none());
        assert!(engine.get_controller(255).is_none());
        engine.shutdown();
    }

    #[test]
    fn test_submit_frame_success() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        let frame = EncodedFrame {
            frame_index: 0,
            nal_data: vec![0u8; 100],
            is_idr: true,
            timestamps: FrameTimestamps::new(0),
        };
        assert!(engine.submit_frame(frame));
        engine.shutdown();
    }

    #[test]
    fn test_submit_frame_channel_full() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        // Channel capacity is 4, so 5th frame should fail
        for i in 0..4 {
            let frame = EncodedFrame {
                frame_index: i,
                nal_data: vec![0u8; 100],
                is_idr: i == 0,
                timestamps: FrameTimestamps::new(i),
            };
            assert!(engine.submit_frame(frame), "frame {} should succeed", i);
        }
        // 5th frame — channel full (receiver not consuming since TCP not connected)
        let frame = EncodedFrame {
            frame_index: 4,
            nal_data: vec![0u8; 100],
            is_idr: false,
            timestamps: FrameTimestamps::new(4),
        };
        assert!(!engine.submit_frame(frame), "frame 4 should fail (channel full)");
        engine.shutdown();
    }
}
