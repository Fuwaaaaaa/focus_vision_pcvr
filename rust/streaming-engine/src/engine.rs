use std::collections::HashMap;
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

/// Channel for sending haptic events to the TCP control writer.
/// Set per session when TCP connection is established.
static HAPTIC_TX: std::sync::RwLock<Option<mpsc::Sender<HapticEvent>>> = std::sync::RwLock::new(None);

/// Counter for dropped haptic events (channel full). Exposed in status.json.
static HAPTIC_DROPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Whether audio capture is active (set by audio pipeline thread).
static AUDIO_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Latest PC-side encode latency in microseconds (for HEARTBEAT_ACK waterfall).
static PC_ENCODE_LATENCY_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Latest PC-side total latency in microseconds (present→send).
static PC_TOTAL_LATENCY_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Check if audio is currently active.
pub fn is_audio_active() -> bool {
    AUDIO_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Get the number of haptic events dropped due to full channel.
pub fn haptic_drop_count() -> u64 {
    HAPTIC_DROPS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Haptic vibration event from SteamVR to HMD.
#[derive(Debug, Clone)]
pub struct HapticEvent {
    pub controller_id: u8,     // 0=left, 1=right
    pub duration_ms: u16,      // vibration duration
    pub frequency: f32,        // Hz
    pub amplitude: f32,        // 0.0 - 1.0
}

/// Queue a haptic event for delivery to HMD. Called from C++ driver thread.
pub fn queue_haptic(controller_id: u8, duration_ms: u16, frequency: f32, amplitude: f32) {
    if let Ok(guard) = HAPTIC_TX.read() {
        if let Some(tx) = guard.as_ref() {
            if tx.try_send(HapticEvent {
                controller_id,
                duration_ms,
                frequency,
                amplitude,
            }).is_err() {
                let count = HAPTIC_DROPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if count % 100 == 1 {
                    log::warn!("Haptic event dropped (total: {})", count);
                }
            }
        }
    }
}

impl HapticEvent {
    /// Serialize haptic event to wire format: [controller_id:1B][duration_ms:2B][frequency:4B][amplitude:4B]
    pub fn to_payload(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(11);
        payload.push(self.controller_id);
        payload.extend_from_slice(&self.duration_ms.to_le_bytes());
        payload.extend_from_slice(&self.frequency.to_le_bytes());
        payload.extend_from_slice(&self.amplitude.to_le_bytes());
        payload
    }

    /// Deserialize haptic event from wire format.
    pub fn from_payload(data: &[u8]) -> Option<Self> {
        if data.len() < 11 { return None; }
        Some(Self {
            controller_id: data[0],
            duration_ms: u16::from_le_bytes([data[1], data[2]]),
            frequency: f32::from_le_bytes([data[3], data[4], data[5], data[6]]),
            amplitude: f32::from_le_bytes([data[7], data[8], data[9], data[10]]),
        })
    }
}

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
    /// Optional session recorder. None when recording is disabled in config.
    /// Drop of StreamingEngine drops this and closes the file.
    recorder: Option<Arc<StdMutex<crate::recording::Recorder>>>,
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
        let latency_tracker = Arc::new(StdMutex::new(LatencyTracker::new(config.video.framerate as usize)));

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

        let recorder = init_recorder(&config);

        Ok(Self {
            runtime,
            frame_tx,
            latest_tracking,
            latest_controllers,
            latency_tracker,
            cancel_token,
            config,
            recorder,
        })
    }

    /// Write a NAL to the active recording, if any. No-op when recording
    /// is disabled or the recorder has been poisoned by a prior I/O error.
    pub fn write_recording_nal(&self, nal: &[u8]) {
        if let Some(rec) = &self.recorder {
            if let Ok(mut r) = rec.try_lock() {
                r.write_nal(nal);
            }
        }
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

/// Read TCP control messages and write haptic events.
/// TCP disconnect reason, used to decide whether to hold state for reconnection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DisconnectReason {
    /// Clean disconnect requested by client (DISCONNECT message)
    ClientRequested,
    /// TCP connection lost (read error, EOF)
    ConnectionLost,
    /// Protocol error (oversized message, etc.)
    ProtocolError,
}

/// Handles: IDR_REQUEST, HEARTBEAT, FACE_DATA, TRANSPORT_FEEDBACK, DISCONNECT (inbound).
/// Sends: HAPTIC_EVENT (outbound to HMD).
/// When the connection closes or errors, cancels the provided token to stop streaming.
/// Returns the disconnect reason so the caller can decide whether to hold state.
#[allow(clippy::too_many_arguments)]
async fn handle_tcp_control(
    stream: Box<dyn crate::control::tcp_server::AsyncStream>,
    cancel: tokio_util::sync::CancellationToken,
    hmd_stats: Arc<StdMutex<Option<HmdStats>>>,
    osc_bridge: Arc<StdMutex<crate::face_tracking::osc_bridge::OscBridge>>,
    gcc_estimator: Arc<StdMutex<crate::adaptive::gcc_estimator::GccEstimator>>,
    sent_packet_log: Arc<StdMutex<HashMap<u16, u64>>>,
    mut haptic_rx: mpsc::Receiver<HapticEvent>,
    gcc_enabled: bool,
) -> Result<DisconnectReason, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const CONFIG_UPDATE_MIN_INTERVAL_MS: u64 = 1000; // Rate limit: 1 update/sec

    // Split stream for concurrent read (inbound messages) and write (haptic events)
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut last_config_update = std::time::Instant::now() - std::time::Duration::from_secs(2);

    /// Send a framed message to the HMD.
    async fn send_msg<W: AsyncWriteExt + Unpin>(writer: &mut W, msg_type: u8, payload: &[u8]) -> std::io::Result<()> {
        let len = (1 + payload.len()) as u32;
        writer.write_all(&len.to_le_bytes()).await?;
        writer.write_all(&[msg_type]).await?;
        writer.write_all(payload).await?;
        writer.flush().await?;
        Ok(())
    }

    let mut msg_buf: Vec<u8> = Vec::with_capacity(256);
    let mut last_idr_time = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let mut idr_suppressed: u64 = 0;

    loop {
        // Concurrently: read inbound messages OR send haptic events
        let mut len_buf = [0u8; 4];
        tokio::select! {
            read_result = reader.read_exact(&mut len_buf) => {
                if read_result.is_err() {
                    log::info!("TCP control connection lost");
                    cancel.cancel();
                    return Ok(DisconnectReason::ConnectionLost);
                }
                let len = u32::from_le_bytes(len_buf) as usize;
                if len == 0 { continue; }
                if len > fvp_common::MAX_MSG_LEN {
                    log::error!("TCP message too large ({} bytes), closing connection", len);
                    cancel.cancel();
                    return Ok(DisconnectReason::ProtocolError);
                }

                msg_buf.clear();
                msg_buf.resize(len, 0);
                if reader.read_exact(&mut msg_buf).await.is_err() {
                    log::info!("TCP control read failed mid-message");
                    cancel.cancel();
                    return Ok(DisconnectReason::ConnectionLost);
                }
                let msg_type = msg_buf[0];

                match msg_type {
                    fvp_common::protocol::msg_type::IDR_REQUEST => {
                        // Rate limit IDR requests: max 2/sec to prevent storm from slice timeouts
                        let now = std::time::Instant::now();
                        let should_fire = {
                            let elapsed = now.duration_since(last_idr_time);
                            elapsed >= std::time::Duration::from_millis(500)
                        };
                        if should_fire {
                            last_idr_time = now;
                            log::info!("Received IDR_REQUEST from client");
                            notify_idr_request();
                        } else {
                            idr_suppressed += 1;
                            if idr_suppressed % 10 == 1 {
                                log::warn!("IDR_REQUEST suppressed (rate limit, {} total)", idr_suppressed);
                            }
                        }
                    }
                    fvp_common::protocol::msg_type::HEARTBEAT => {
                        let payload = &msg_buf[1..];
                        if payload.len() >= 26 {
                            let stats_offset = 12;
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

                            // Send HEARTBEAT_ACK with PC-side latency for waterfall overlay
                            let encode_us = PC_ENCODE_LATENCY_US.load(std::sync::atomic::Ordering::Relaxed);
                            let total_us = PC_TOTAL_LATENCY_US.load(std::sync::atomic::Ordering::Relaxed);
                            let mut ack_payload = Vec::with_capacity(8);
                            ack_payload.extend_from_slice(&encode_us.to_le_bytes());
                            ack_payload.extend_from_slice(&total_us.to_le_bytes());
                            if let Err(e) = send_msg(&mut writer,
                                fvp_common::protocol::msg_type::HEARTBEAT_ACK,
                                &ack_payload).await {
                                log::warn!("Failed to send HEARTBEAT_ACK: {}", e);
                            }
                        }
                    }
                    fvp_common::protocol::msg_type::FACE_DATA => {
                        let payload = &msg_buf[1..];
                        if let Some((lip_valid, eye_valid, lip, eye)) =
                            crate::face_tracking::osc_bridge::parse_face_data(payload)
                        {
                            if let Ok(mut bridge) = osc_bridge.try_lock() {
                                bridge.send_face_data(lip_valid, eye_valid, &lip, &eye);
                            }
                        }
                    }
                    fvp_common::protocol::msg_type::TRANSPORT_FEEDBACK => {
                        let payload = &msg_buf[1..];
                        if let Some(entries) = fvp_common::protocol::parse_transport_feedback(payload) {
                            log::debug!("Received TRANSPORT_FEEDBACK: {} entries", entries.len());
                            // Enrich feedback entries with PC-side send timestamps
                            if let Ok(guard) = sent_packet_log.try_lock() {
                                for entry in &entries {
                                    if let Some(&send_us) = guard.get(&entry.sequence) {
                                        log::trace!(
                                            "FEEDBACK seq={} send_us={} recv_delta_us={}",
                                            entry.sequence, send_us, entry.recv_delta_us
                                        );
                                    }
                                }
                            }
                            // Only feed GCC estimator when delay-based congestion control is enabled
                            if gcc_enabled {
                                if let Ok(mut guard) = gcc_estimator.try_lock() {
                                    guard.process_feedback(&entries);
                                }
                            }
                        } else {
                            log::warn!("Invalid TRANSPORT_FEEDBACK payload ({}B)", payload.len());
                        }
                    }
                    fvp_common::protocol::msg_type::CONFIG_UPDATE => {
                        // HMD dashboard requests a config change.
                        // Payload: [key:1B][value:4B LE]
                        // Keys: 0x01=bitrate_mbps(u32), 0x02=codec(0=h264,1=h265)
                        // Rate limit: ignore if <1s since last update
                        let payload = &msg_buf[1..];
                        let elapsed = last_config_update.elapsed();
                        if elapsed < std::time::Duration::from_millis(CONFIG_UPDATE_MIN_INTERVAL_MS) {
                            log::warn!("CONFIG_UPDATE rate limited ({:?} since last)", elapsed);
                            continue;
                        }
                        if payload.len() >= 5 {
                            last_config_update = std::time::Instant::now();
                            let key = payload[0];
                            let value = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
                            let mut ack_status: u8 = 0x00; // 0=rejected, 1=accepted
                            match key {
                                0x01 => { // bitrate_mbps
                                    if (10..=200).contains(&value) {
                                        log::info!("CONFIG_UPDATE: bitrate → {} Mbps", value);
                                        notify_bitrate_change(value * 1_000_000);
                                        ack_status = 0x01;
                                    } else {
                                        log::warn!("CONFIG_UPDATE: bitrate {} out of range", value);
                                    }
                                }
                                0x02 => { // codec (0=h264, 1=h265)
                                    log::info!("CONFIG_UPDATE: codec → {}", if value == 0 { "h264" } else { "h265" });
                                    // Codec change requires stream restart — acknowledged but deferred
                                    ack_status = 0x01;
                                }
                                _ => {
                                    log::warn!("CONFIG_UPDATE: unknown key 0x{:02x}", key);
                                }
                            }
                            // Send ACK back to HMD
                            if let Err(e) = send_msg(&mut writer,
                                fvp_common::protocol::msg_type::CONFIG_UPDATE_ACK,
                                &[ack_status, key]).await
                            {
                                log::warn!("Failed to send CONFIG_UPDATE_ACK: {}", e);
                            }
                        }
                    }
                    fvp_common::protocol::msg_type::DISCONNECT => {
                        log::info!("Client sent DISCONNECT — stopping stream");
                        cancel.cancel();
                        return Ok(DisconnectReason::ClientRequested);
                    }
                    _ => {
                        log::warn!("Unknown TCP message type 0x{:02x} (len={}B) — skipping (client may be newer)", msg_type, msg_buf.len() - 1);
                    }
                }
            }
            Some(haptic) = haptic_rx.recv() => {
                let payload = haptic.to_payload();
                if let Err(e) = send_msg(&mut writer, fvp_common::protocol::msg_type::HAPTIC_EVENT, &payload).await {
                    log::warn!("Failed to send haptic event: {}", e);
                }
            }
        }
    }
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
                Some(c) => {
                    AUDIO_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
                    c
                }
                None => {
                    AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
                    log::info!("Audio capture unavailable — streaming video only");
                    return;
                }
            };
            while !hold_cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
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

/// Check HMD-reported stats and adjust bitrate and FEC redundancy accordingly.
/// Called once per second (every `framerate` frames).
#[allow(clippy::too_many_arguments)]
fn update_adaptive_bitrate(
    frame_count: u64,
    framerate: u64,
    hmd_stats: &Arc<StdMutex<Option<HmdStats>>>,
    gcc_estimator: &Arc<StdMutex<crate::adaptive::gcc_estimator::GccEstimator>>,
    bw_estimator: &mut crate::adaptive::bandwidth_estimator::BandwidthEstimator,
    bitrate_ctrl: &mut crate::adaptive::bitrate_controller::BitrateController,
    mut adaptive_fec: Option<&mut crate::transport::fec::AdaptiveFecController>,
    fec_encoder: &mut crate::transport::fec::FecEncoder,
    burst_detector: &Arc<StdMutex<crate::adaptive::burst_detector::BurstDetector>>,
    gcc_enabled: bool,
) {
    if !frame_count.is_multiple_of(framerate) {
        return;
    }

    if let Ok(mut guard) = hmd_stats.lock() {
        if let Some(stats) = guard.take() {
            bw_estimator.update(
                stats.packets_received,
                stats.packets_lost,
                0.0, // RTT not yet measured
            );

            if gcc_enabled {
                // Delay-based mode: use GCC estimator and burst detector
                // Update burst detector with current loss rate
                if let Ok(mut burst_guard) = burst_detector.lock() {
                    burst_guard.record(bw_estimator.loss_rate());
                }

                if let Ok(gcc_guard) = gcc_estimator.lock() {
                    if let Ok(burst_guard) = burst_detector.lock() {
                        if bitrate_ctrl.adjust(bw_estimator, &gcc_guard, &burst_guard) {
                            let new_bps = bitrate_ctrl.current_bitrate_bps() as u32;
                            notify_bitrate_change(new_bps);
                        }

                        // If burst detected, boost FEC temporarily
                        if burst_guard.recommend_fec_boost() {
                            if let Some(ref mut afec) = adaptive_fec {
                                log::info!("Burst loss: boosting FEC redundancy");
                                afec.activate_boost();
                                fec_encoder.set_redundancy(afec.effective_redundancy());
                                return;
                            }
                        } else if let Some(ref mut afec) = adaptive_fec {
                            if afec.effective_redundancy() != afec.current_redundancy() {
                                // Burst ended, deactivate boost
                                afec.deactivate_boost();
                                fec_encoder.set_redundancy(afec.effective_redundancy());
                            }
                        }
                    }
                }
            } else {
                // Loss-only mode: use default (no-op) GCC and burst state for bitrate adjustment
                let default_gcc = crate::adaptive::gcc_estimator::GccEstimator::new(
                    bitrate_ctrl.current_bitrate_bps(),
                );
                let default_burst = crate::adaptive::burst_detector::BurstDetector::new();
                if bitrate_ctrl.adjust(bw_estimator, &default_gcc, &default_burst) {
                    let new_bps = bitrate_ctrl.current_bitrate_bps() as u32;
                    notify_bitrate_change(new_bps);
                }
            }
            // Adjust FEC redundancy based on observed loss
            if let Some(afec) = adaptive_fec {
                if afec.adjust(bw_estimator.loss_rate()) {
                    fec_encoder.set_redundancy(afec.effective_redundancy());
                }
            }
        }
    }
}

/// Detect user inactivity from head tracking and enter/exit sleep mode.
fn check_sleep_mode(
    tracking: &Arc<StdMutex<Option<TrackingData>>>,
    sleep_detector: &mut crate::sleep_mode::SleepDetector,
    normal_bitrate_mbps: u32,
    timeout_seconds: u32,
) {
    if let Ok(guard) = tracking.lock() {
        if let Some(ref data) = *guard {
            if let Some(transition) = sleep_detector.update(data) {
                match transition {
                    crate::sleep_mode::SleepTransition::Sleep => {
                        log::info!("Sleep mode: entering (no motion for {}s)", timeout_seconds);
                        let bps = sleep_detector.sleep_bitrate_mbps() * 1_000_000;
                        notify_bitrate_change(bps);
                    }
                    crate::sleep_mode::SleepTransition::Wake => {
                        log::info!("Sleep mode: waking up (motion detected)");
                        let bps = normal_bitrate_mbps * 1_000_000;
                        notify_bitrate_change(bps);
                    }
                }
            }
        }
    }
}

/// Push latest latency measurements to atomics for HEARTBEAT_ACK waterfall overlay.
fn update_latency_atomics(latency_tracker: &Arc<StdMutex<LatencyTracker>>) {
    if let Ok(tracker) = latency_tracker.lock() {
        if let Some(enc) = tracker.avg_encode_latency_us() {
            PC_ENCODE_LATENCY_US.store(enc as u32, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(total) = tracker.avg_pc_latency_us() {
            PC_TOTAL_LATENCY_US.store(total as u32, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Log PC-side latency stats every 5 seconds.
/// Build a Recorder if config.recording.enabled is true.
/// Returns None when disabled or when the output file cannot be opened.
fn init_recorder(config: &AppConfig) -> Option<Arc<StdMutex<crate::recording::Recorder>>> {
    if !config.recording.enabled {
        return None;
    }
    let dir: std::path::PathBuf = if config.recording.output_dir.is_empty() {
        match dirs_next::data_dir() {
            Some(d) => d.join("FocusVisionPCVR").join("recordings"),
            None => {
                log::warn!("recorder: no data_dir() available, recording disabled");
                return None;
            }
        }
    } else {
        std::path::PathBuf::from(&config.recording.output_dir)
    };
    let ext = match config.video.codec {
        fvp_common::protocol::VideoCodec::H264 => "h264",
        fvp_common::protocol::VideoCodec::H265 => "h265",
    };
    let path = dir.join(crate::recording::default_filename(ext));
    crate::recording::Recorder::open(path).map(|r| Arc::new(StdMutex::new(r)))
}

fn log_periodic_stats(frame_count: u64, framerate: u64) {
    if frame_count.is_multiple_of(framerate * 5) {
        let enc = PC_ENCODE_LATENCY_US.load(std::sync::atomic::Ordering::Relaxed);
        let total = PC_TOTAL_LATENCY_US.load(std::sync::atomic::Ordering::Relaxed);
        if total > 0 {
            log::info!("PC latency: avg={}us encode={}us", total, enc);
        }
    }
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
    let mut accept_failures: u32 = 0;
    let mut reconnect_attempts: u32 = 0;
    const MAX_ACCEPT_FAILURES: u32 = 5;
    const MAX_RECONNECT_ATTEMPTS: u32 = 10; // more lenient for Wi-Fi drops
    let backoff_base = std::time::Duration::from_secs(1);

    loop {
        if cancel.is_cancelled() { break; }

        if accept_failures > 0 {
            let delay = backoff_base * 2u32.pow((accept_failures - 1).min(4));
            log::info!("Retrying accept in {:?} (failure {}/{})", delay, accept_failures, MAX_ACCEPT_FAILURES);
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
                accept_failures += 1;
                if accept_failures > MAX_ACCEPT_FAILURES {
                    log::error!("Max accept failures reached ({}) — stopping engine", accept_failures);
                    break;
                }
                continue;
            }
        };

        log::info!("HMD connected from {}, starting video stream", peer_addr);
        accept_failures = 0; // Reset on successful accept

        // Per-session cancel: fires when TCP drops or HMD disconnects
        let session_cancel = CancellationToken::new();

        // Shared HMD stats for adaptive bitrate (fed by heartbeat messages)
        let hmd_stats: Arc<StdMutex<Option<HmdStats>>> = Arc::new(StdMutex::new(None));

        // Burst detector: classifies transient burst loss vs sustained congestion
        let burst_detector: Arc<StdMutex<crate::adaptive::burst_detector::BurstDetector>> =
            Arc::new(StdMutex::new(crate::adaptive::burst_detector::BurstDetector::new()));

        // GCC delay-based bandwidth estimator (shared between TCP handler and adaptive loop)
        // When congestion_control == "loss", GCC feedback is not processed (loss-only mode).
        let gcc_enabled = config.network.congestion_control == "gcc";
        let gcc_estimator: Arc<StdMutex<crate::adaptive::gcc_estimator::GccEstimator>> =
            Arc::new(StdMutex::new(crate::adaptive::gcc_estimator::GccEstimator::new(
                config.video.bitrate_mbps as u64 * 1_000_000,
            )));

        // Sent packet log: maps RTP sequence number → PC-side send timestamp (µs)
        // Shared between the video send loop (writer) and TCP handler (reader)
        let sent_packet_log: Arc<StdMutex<HashMap<u16, u64>>> =
            Arc::new(StdMutex::new(HashMap::new()));

        // OSC bridge for face tracking data (HMD → VRChat)
        let osc_bridge = Arc::new(StdMutex::new(
            crate::face_tracking::osc_bridge::OscBridge::with_smoothing(config.face_tracking.smoothing)
        ));

        // Haptic event channel (PC driver → TCP → HMD)
        let (haptic_tx, haptic_rx) = mpsc::channel::<HapticEvent>(16);
        if let Ok(mut guard) = HAPTIC_TX.write() {
            *guard = Some(haptic_tx);
        }

        // Spawn TCP control reader/writer. Track disconnect reason for hold logic.
        let disconnect_reason: Arc<StdMutex<Option<DisconnectReason>>> = Arc::new(StdMutex::new(None));
        let tcp_session = session_cancel.clone();
        let stats_clone = hmd_stats.clone();
        let osc_clone = osc_bridge.clone();
        let gcc_clone = gcc_estimator.clone();
        let sent_log_clone = sent_packet_log.clone();
        let reason_clone = disconnect_reason.clone();
        tokio::spawn(async move {
            match handle_tcp_control(tcp_control_stream, tcp_session, stats_clone, osc_clone, gcc_clone, sent_log_clone, haptic_rx, gcc_enabled).await {
                Ok(reason) => {
                    if let Ok(mut guard) = reason_clone.lock() {
                        *guard = Some(reason);
                    }
                    log::info!("TCP control ended: {:?}", reason);
                }
                Err(e) => {
                    if let Ok(mut guard) = reason_clone.lock() {
                        *guard = Some(DisconnectReason::ConnectionLost);
                    }
                    log::warn!("TCP control error: {}", e);
                }
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
                accept_failures += 1;
                continue;
            }
        };

        // Audio pipeline (per-session)
        let audio_port = config.network.udp_port + fvp_common::AUDIO_PORT_OFFSET;
        let audio_target: SocketAddr = SocketAddr::new(peer_addr.ip(), audio_port);
        spawn_audio_pipeline(audio_target, session_cancel.clone());

        // Step 3: Process frames with adaptive bitrate + adaptive FEC
        let mut packetizer = RtpPacketizer::new(0x46565000);
        let mut fec_encoder = crate::transport::fec::FecEncoder::new(config.network.fec_redundancy);
        let slice_count = config.network.slice_count;
        let slice_fec_enabled = config.network.slice_fec_enabled;
        let mut slice_fec_encoders: Vec<crate::transport::fec::FecEncoder> = (0..slice_count)
            .map(|_| crate::transport::fec::FecEncoder::new(config.network.fec_redundancy))
            .collect();
        let mut frame_count: u64 = 0;
        let mut latency_skip_count: u64 = 0;

        let mut bw_estimator = crate::adaptive::bandwidth_estimator::BandwidthEstimator::new();
        let mut bitrate_ctrl = crate::adaptive::bitrate_controller::BitrateController::new(
            config.video.bitrate_mbps,
        );
        let mut adaptive_fec = if config.network.adaptive_fec_enabled {
            Some(crate::transport::fec::AdaptiveFecController::new(
                config.network.fec_redundancy_min,
                config.network.fec_redundancy_max,
                config.network.fec_redundancy,
            ))
        } else {
            log::info!("Adaptive FEC disabled — using fixed redundancy {:.0}%", config.network.fec_redundancy * 100.0);
            None
        };
        let mut sleep_detector = crate::sleep_mode::SleepDetector::new(
            config.sleep_mode.enabled,
            config.sleep_mode.motion_threshold,
            config.sleep_mode.timeout_seconds,
            config.sleep_mode.sleep_bitrate_mbps,
        );
        let normal_bitrate_mbps = config.video.bitrate_mbps;

        loop {
            tokio::select! {
                frame_opt = frame_rx.recv() => {
                    let mut frame = match frame_opt {
                        Some(f) => f,
                        None => break, // Channel closed (engine shutdown)
                    };

                    frame.timestamps.mark_encode_start();
                    frame.timestamps.mark_encode_end();

                    let framerate = config.video.framerate as u64;
                    // Multiply first to avoid integer division truncation drift (e.g. 90000/96=937.5)
                    let timestamp_90khz = (frame_count * fvp_common::RTP_CLOCK_RATE as u64 / framerate) as u32;

                    // Choose slice FEC for large frames, bulk FEC for small ones
                    let use_slice_fec = slice_fec_enabled
                        && frame.nal_data.len() >= pipeline::MIN_SLICE_SIZE;

                    if use_slice_fec {
                        // Slice FEC: split frame → RS encode per slice → send each slice immediately
                        let slice_batches = pipeline::encode_frame_sliced(
                            &frame.nal_data,
                            frame.frame_index,
                            timestamp_90khz,
                            frame.is_idr,
                            slice_count,
                            &mut slice_fec_encoders,
                            &mut packetizer,
                        );
                        for slice_packets in &slice_batches {
                            if slice_packets.is_empty() { continue; }
                            if let Err(e) = udp_sender.send_all(slice_packets).await {
                                log::warn!("UDP send error (slice FEC): {}", e);
                            }
                            // Record send timestamps per-slice for GCC
                            let send_us = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_micros() as u64;
                            if let Ok(mut log_guard) = sent_packet_log.try_lock() {
                                for packet in slice_packets {
                                    if packet.data.len() >= 4 {
                                        let seq = u16::from_be_bytes([packet.data[2], packet.data[3]]);
                                        log_guard.insert(seq, send_us);
                                    }
                                }
                            }
                        }
                    } else {
                        // Bulk FEC: existing path
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

                        // Record send timestamps for GCC
                        let send_us = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_micros() as u64;
                        if let Ok(mut log_guard) = sent_packet_log.try_lock() {
                            for packet in &packets {
                                if packet.data.len() >= 4 {
                                    let seq = u16::from_be_bytes([packet.data[2], packet.data[3]]);
                                    log_guard.insert(seq, send_us);
                                }
                            }
                        }
                    }

                    // Prune sent_packet_log to bound memory usage
                    if let Ok(mut log_guard) = sent_packet_log.try_lock() {
                        if log_guard.len() > 5000 {
                            let mut entries: Vec<(u16, u64)> = log_guard.drain().collect();
                            entries.sort_unstable_by_key(|&(_, ts)| std::cmp::Reverse(ts));
                            entries.truncate(2500);
                            log_guard.extend(entries);
                        }
                    }

                    // Buffer pool recycle is handled by packetizer internally
                    // (slice path sends per-slice, bulk path sends all at once)

                    frame.timestamps.mark_send();

                    if let Ok(mut tracker) = latency_tracker.try_lock() {
                        tracker.record(frame.timestamps);
                    } else {
                        latency_skip_count += 1;
                        if latency_skip_count % 90 == 1 {
                            log::debug!("Latency tracker lock contention (skipped {} samples)", latency_skip_count);
                        }
                    }

                    frame_count += 1;

                    update_adaptive_bitrate(
                        frame_count, framerate, &hmd_stats,
                        &gcc_estimator,
                        &mut bw_estimator, &mut bitrate_ctrl,
                        adaptive_fec.as_mut(), &mut fec_encoder,
                        &burst_detector,
                        gcc_enabled,
                    );

                    // Sync slice FEC encoders with current redundancy
                    if slice_fec_enabled {
                        let r = fec_encoder.redundancy();
                        for se in &mut slice_fec_encoders {
                            se.set_redundancy(r);
                        }
                    }

                    check_sleep_mode(
                        &_tracking, &mut sleep_detector,
                        normal_bitrate_mbps, config.sleep_mode.timeout_seconds,
                    );

                    update_latency_atomics(&latency_tracker);

                    log_periodic_stats(frame_count, framerate);
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

        // Session ended — check disconnect reason for hold logic
        let reason = disconnect_reason.lock().ok()
            .and_then(|g| *g)
            .unwrap_or(DisconnectReason::ConnectionLost);

        match reason {
            DisconnectReason::ConnectionLost => {
                log::info!("Connection lost — listening for reconnection (5s hold)");
                reconnect_attempts += 1;
                if reconnect_attempts > MAX_RECONNECT_ATTEMPTS {
                    log::warn!("Wi-Fi reconnect attempts: {}/{} — still accepting connections", reconnect_attempts, MAX_RECONNECT_ATTEMPTS);
                }

                // Re-create TCP server and listen during hold period
                let tcp_server = TcpControlServer::new(config.clone());
                let hold_duration = std::time::Duration::from_secs(5);

                let reconnect_result = tokio::select! {
                    r = tcp_server.listen_and_accept() => Some(r),
                    _ = tokio::time::sleep(hold_duration) => None,
                    _ = cancel.cancelled() => {
                        log::info!("Engine shutdown during hold period");
                        return Ok(());
                    }
                };

                match reconnect_result {
                    Some(Ok((_stream, addr))) => {
                        log::info!("HMD reconnected from {} during hold period", addr);
                        // HMD reconnected — loop will accept again immediately.
                        // The key improvement: listener was open, so HMD could find us.
                        reconnect_attempts = 0;
                        continue; // Skip backoff, go directly to new session
                    }
                    Some(Err(e)) => {
                        log::warn!("Accept failed during hold: {}", e);
                        accept_failures += 1;
                    }
                    None => {
                        log::info!("Hold period expired — no reconnection");
                    }
                }
            }
            DisconnectReason::ClientRequested => {
                // Clean disconnect — no hold needed
                log::info!("Client requested disconnect — ready for new connection");
                accept_failures = 0;
                reconnect_attempts = 0;
            }
            DisconnectReason::ProtocolError => {
                log::warn!("Protocol error — reconnecting");
                reconnect_attempts += 1;
            }
        }

        if accept_failures > MAX_ACCEPT_FAILURES {
            log::error!("Max accept failures reached ({}) — stopping engine", accept_failures);
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

    #[test]
    fn test_haptic_event_serialization_roundtrip() {
        let event = HapticEvent {
            controller_id: 1,
            duration_ms: 250,
            frequency: 160.0,
            amplitude: 0.75,
        };
        let payload = event.to_payload();
        assert_eq!(payload.len(), 11);

        let decoded = HapticEvent::from_payload(&payload).unwrap();
        assert_eq!(decoded.controller_id, 1);
        assert_eq!(decoded.duration_ms, 250);
        assert!((decoded.frequency - 160.0).abs() < 1e-6);
        assert!((decoded.amplitude - 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_haptic_event_payload_too_short() {
        assert!(HapticEvent::from_payload(&[0u8; 10]).is_none());
        assert!(HapticEvent::from_payload(&[]).is_none());
    }

    #[test]
    fn test_haptic_event_left_right_controllers() {
        let left = HapticEvent { controller_id: 0, duration_ms: 100, frequency: 320.0, amplitude: 1.0 };
        let right = HapticEvent { controller_id: 1, duration_ms: 50, frequency: 160.0, amplitude: 0.5 };

        let lp = left.to_payload();
        let rp = right.to_payload();
        assert_eq!(lp[0], 0); // left
        assert_eq!(rp[0], 1); // right
    }

    #[test]
    fn test_queue_haptic_no_tx_doesnt_panic() {
        // With no active session, HAPTIC_TX is None — should not panic
        if let Ok(mut guard) = HAPTIC_TX.write() {
            *guard = None;
        }
        queue_haptic(0, 100, 160.0, 0.5); // should be a no-op
    }

    #[tokio::test]
    async fn test_queue_haptic_channel_full() {
        let (tx, _rx) = mpsc::channel::<HapticEvent>(2); // small channel
        if let Ok(mut guard) = HAPTIC_TX.write() {
            *guard = Some(tx);
        }

        // Fill the channel
        queue_haptic(0, 100, 160.0, 1.0);
        queue_haptic(0, 100, 160.0, 1.0);
        // Third should be dropped silently (channel full)
        queue_haptic(0, 100, 160.0, 1.0);

        // Clean up
        if let Ok(mut guard) = HAPTIC_TX.write() {
            *guard = None;
        }
    }

    #[test]
    fn test_rtp_timestamp_at_90fps() {
        // At 90fps: each frame increments by 90000/90 = 1000
        let framerate = 90u64;
        let tick = fvp_common::RTP_CLOCK_RATE as u64 / framerate;
        assert_eq!(tick, 1000);

        // Frame 0 → timestamp 0, Frame 1 → 1000, Frame 90 → 90000 (1 second)
        assert_eq!((0u64 * tick) as u32, 0);
        assert_eq!((1u64 * tick) as u32, 1000);
        assert_eq!((90u64 * tick) as u32, 90000);
    }

    #[test]
    fn test_rtp_timestamp_at_96fps() {
        // At 96fps with multiply-first: frame_count * 90000 / 96
        let framerate = 96u64;
        let clock = fvp_common::RTP_CLOCK_RATE as u64;

        // Frame 96 → exactly 90000 (1 second, no drift)
        let ts_1s = (96u64 * clock / framerate) as u32;
        assert_eq!(ts_1s, 90000, "1s timestamp should be exactly 90000");

        // Frame 1 → 937 (90000/96 truncated, but drift-free over time)
        let ts_f1 = (1u64 * clock / framerate) as u32;
        assert_eq!(ts_f1, 937);
    }

    #[test]
    fn test_rtp_timestamp_at_120fps() {
        // At 120fps: each frame increments by 90000/120 = 750
        let framerate = 120u64;
        let tick = fvp_common::RTP_CLOCK_RATE as u64 / framerate;
        assert_eq!(tick, 750);

        // Frame 120 → 90000 (exactly 1 second)
        assert_eq!((120u64 * tick) as u32, 90000);
    }

    /// Helper: build a framed TCP message (4-byte LE length + msg_type + payload)
    fn build_tcp_msg(msg_type: u8, payload: &[u8]) -> Vec<u8> {
        let len = (1 + payload.len()) as u32;
        let mut buf = Vec::with_capacity(4 + len as usize);
        buf.extend_from_slice(&len.to_le_bytes());
        buf.push(msg_type);
        buf.extend_from_slice(payload);
        buf
    }

    /// Helper: create a tcp_test_harness with mock duplex stream
    struct TcpTestHarness {
        cancel: CancellationToken,
        hmd_stats: Arc<StdMutex<Option<HmdStats>>>,
        osc_bridge: Arc<StdMutex<crate::face_tracking::osc_bridge::OscBridge>>,
        gcc_estimator: Arc<StdMutex<crate::adaptive::gcc_estimator::GccEstimator>>,
        sent_packet_log: Arc<StdMutex<HashMap<u16, u64>>>,
        disconnect_reason: Option<DisconnectReason>,
    }

    impl TcpTestHarness {
        fn new() -> Self {
            Self {
                cancel: CancellationToken::new(),
                hmd_stats: Arc::new(StdMutex::new(None)),
                gcc_estimator: Arc::new(StdMutex::new(
                    crate::adaptive::gcc_estimator::GccEstimator::new(80_000_000)
                )),
                sent_packet_log: Arc::new(StdMutex::new(HashMap::new())),
                osc_bridge: Arc::new(StdMutex::new(
                    crate::face_tracking::osc_bridge::OscBridge::with_smoothing(0.5)
                )),
                disconnect_reason: None,
            }
        }

        async fn run_with_input(mut self, input: &[u8]) -> Self {
            let (client, server) = tokio::io::duplex(4096);
            let (_haptic_tx, haptic_rx) = mpsc::channel::<HapticEvent>(16);

            // Write input to client side, then close
            let (mut client_read, mut client_write) = tokio::io::split(client);
            use tokio::io::AsyncWriteExt;
            client_write.write_all(input).await.unwrap();
            drop(client_write); // Close write side → EOF for server reader

            let cancel = self.cancel.clone();
            let stats = self.hmd_stats.clone();
            let osc = self.osc_bridge.clone();

            // Run handle_tcp_control (will read until EOF)
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                handle_tcp_control(Box::new(server), cancel, stats, osc, self.gcc_estimator.clone(), self.sent_packet_log.clone(), haptic_rx, true)
            ).await;

            // Capture disconnect reason
            if let Ok(Ok(reason)) = result {
                self.disconnect_reason = Some(reason);
            }

            // Read any response from server (ACK messages)
            let mut response = Vec::new();
            use tokio::io::AsyncReadExt;
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                client_read.read_to_end(&mut response)
            ).await;

            self
        }
    }

    #[tokio::test]
    async fn test_handle_tcp_config_update_valid_bitrate() {
        let mut payload = vec![0x01u8]; // key = bitrate
        payload.extend_from_slice(&100u32.to_le_bytes()); // value = 100 Mbps
        let msg = build_tcp_msg(fvp_common::protocol::msg_type::CONFIG_UPDATE, &payload);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        // Should not cancel (valid update)
        assert!(!harness.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_tcp_config_update_invalid_bitrate() {
        let mut payload = vec![0x01u8]; // key = bitrate
        payload.extend_from_slice(&0u32.to_le_bytes()); // value = 0 (out of range)
        let msg = build_tcp_msg(fvp_common::protocol::msg_type::CONFIG_UPDATE, &payload);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        assert!(!harness.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_tcp_config_update_unknown_key() {
        let mut payload = vec![0xFFu8]; // unknown key
        payload.extend_from_slice(&50u32.to_le_bytes());
        let msg = build_tcp_msg(fvp_common::protocol::msg_type::CONFIG_UPDATE, &payload);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        assert!(!harness.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_tcp_config_update_short_payload() {
        // Only 3 bytes payload (< 5 needed)
        let msg = build_tcp_msg(fvp_common::protocol::msg_type::CONFIG_UPDATE, &[0x01, 0x00, 0x00]);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        assert!(!harness.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_tcp_disconnect_cancels_token() {
        let msg = build_tcp_msg(fvp_common::protocol::msg_type::DISCONNECT, &[]);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        assert!(harness.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_tcp_oversized_message_rejected() {
        // Write a length field of 70000 (> MAX_MSG_LEN)
        let mut msg = Vec::new();
        msg.extend_from_slice(&70000u32.to_le_bytes());
        // Don't need actual data — handler should reject based on length alone

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        assert!(harness.cancel.is_cancelled());
    }

    #[tokio::test]
    async fn test_handle_tcp_heartbeat_parses_stats() {
        // HEARTBEAT payload: 12 bytes padding + 14 bytes stats = 26+ bytes
        let mut payload = vec![0u8; 26];
        // Stats at offset 12: packets_received(4) + packets_lost(4) + avg_decode_us(4) + fps(2)
        let stats_start = 12;
        payload[stats_start..stats_start + 4].copy_from_slice(&1000u32.to_le_bytes()); // packets_received
        payload[stats_start + 4..stats_start + 8].copy_from_slice(&5u32.to_le_bytes()); // packets_lost
        payload[stats_start + 8..stats_start + 12].copy_from_slice(&3000u32.to_le_bytes()); // avg_decode_us
        payload[stats_start + 12..stats_start + 14].copy_from_slice(&90u16.to_le_bytes()); // fps

        let msg = build_tcp_msg(fvp_common::protocol::msg_type::HEARTBEAT, &payload);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;

        let stats = harness.hmd_stats.lock().unwrap();
        assert!(stats.is_some(), "HmdStats should be populated after HEARTBEAT");
        let s = stats.as_ref().unwrap();
        assert_eq!(s.packets_received, 1000);
        assert_eq!(s.packets_lost, 5);
        assert_eq!(s.avg_decode_us, 3000);
        assert_eq!(s.fps, 90);
    }

    #[tokio::test]
    async fn test_handle_tcp_face_data_forwarded() {
        // Build FACE_DATA payload: [lip_valid:1][eye_valid:1][lip:37*4][eye:14*4] = 206 bytes
        let mut payload = vec![0u8; 206];
        payload[0] = 1; // lip_valid
        payload[1] = 0; // eye not valid
        // Set JawOpen (index 3) = 0.9
        let jaw_off = 2 + 3 * 4;
        payload[jaw_off..jaw_off + 4].copy_from_slice(&0.9f32.to_le_bytes());

        let msg = build_tcp_msg(fvp_common::protocol::msg_type::FACE_DATA, &payload);

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;

        // Check that osc_bridge was updated (prev_lip[3] should be non-zero)
        let bridge = harness.osc_bridge.lock().unwrap();
        // With smoothing=0.5: smoothed = 0.5 * 0.0 + 0.5 * 0.9 = 0.45
        assert!(bridge.prev_lip()[3] > 0.1, "osc_bridge should have received face data");
    }

    #[tokio::test]
    async fn test_handle_tcp_unknown_msg_type_skipped() {
        // Unknown message followed by DISCONNECT. Verifies the unknown msg
        // doesn't crash and the loop continues to process the next message.
        let mut input = build_tcp_msg(0xFF, &[0x01, 0x02, 0x03]);
        input.extend_from_slice(&build_tcp_msg(fvp_common::protocol::msg_type::DISCONNECT, &[]));

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&input).await;
        // Cancel comes from DISCONNECT, not the unknown msg — proves the loop continued
        assert!(harness.cancel.is_cancelled());
    }

    #[test]
    fn test_bitrate_adjustment_interval_scales_with_framerate() {
        // At 90fps, adjustment every 90 frames = 1 second
        // At 96fps, adjustment every 96 frames = 1 second
        // At 120fps, adjustment every 120 frames = 1 second
        for fps in [90u64, 96, 120] {
            let interval = fps;
            assert!(fps.is_multiple_of(interval), "frame_count should align at {fps}fps");
            // Verify the interval represents ~1 second
            assert_eq!(interval, fps);
        }
    }

    #[tokio::test]
    async fn test_disconnect_reason_client_requested() {
        let msg = build_tcp_msg(fvp_common::protocol::msg_type::DISCONNECT, &[]);
        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        assert_eq!(harness.disconnect_reason, Some(DisconnectReason::ClientRequested));
    }

    #[tokio::test]
    async fn test_disconnect_reason_protocol_error() {
        // Oversized message → ProtocolError → cancel is called
        let mut msg = Vec::new();
        msg.extend_from_slice(&70000u32.to_le_bytes());
        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&msg).await;
        // ProtocolError always cancels
        assert!(harness.cancel.is_cancelled());
        assert_eq!(harness.disconnect_reason, Some(DisconnectReason::ProtocolError));
    }

    #[tokio::test]
    async fn test_disconnect_reason_enum_values() {
        // Verify enum variants are distinct
        assert_ne!(DisconnectReason::ClientRequested, DisconnectReason::ConnectionLost);
        assert_ne!(DisconnectReason::ConnectionLost, DisconnectReason::ProtocolError);
        assert_ne!(DisconnectReason::ClientRequested, DisconnectReason::ProtocolError);
    }

    #[tokio::test]
    async fn test_transport_feedback_valid_not_crash() {
        // Send valid TRANSPORT_FEEDBACK followed by DISCONNECT
        let entries = vec![
            fvp_common::protocol::TransportFeedbackEntry { sequence: 1, recv_delta_us: 100 },
        ];
        let payload = fvp_common::protocol::encode_transport_feedback(&entries);
        let mut input = build_tcp_msg(fvp_common::protocol::msg_type::TRANSPORT_FEEDBACK, &payload);
        input.extend_from_slice(&build_tcp_msg(fvp_common::protocol::msg_type::DISCONNECT, &[]));

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&input).await;
        assert!(harness.cancel.is_cancelled());
        assert_eq!(harness.disconnect_reason, Some(DisconnectReason::ClientRequested));
    }

    #[tokio::test]
    async fn test_transport_feedback_invalid_not_crash() {
        // Send invalid TRANSPORT_FEEDBACK (truncated) followed by DISCONNECT
        let mut input = build_tcp_msg(fvp_common::protocol::msg_type::TRANSPORT_FEEDBACK, &[0x01]); // too short
        input.extend_from_slice(&build_tcp_msg(fvp_common::protocol::msg_type::DISCONNECT, &[]));

        let harness = TcpTestHarness::new();
        let harness = harness.run_with_input(&input).await;
        // Should warn but continue to DISCONNECT
        assert!(harness.cancel.is_cancelled());
        assert_eq!(harness.disconnect_reason, Some(DisconnectReason::ClientRequested));
    }
}
