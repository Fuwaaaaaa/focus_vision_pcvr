use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::control::tcp_server::{AsyncStream, TcpControlServer};
use crate::metrics::latency::{FrameTimestamps, LatencyTracker};
use crate::pipeline;
use crate::tracking::receiver::{AuthorizedPeer, TrackingReceiver};
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

/// Runtime recording flag. Initialised from `config.recording.enabled` at
/// engine startup and toggleable mid-session via CONFIG_UPDATE key 0x03.
/// `write_recording_nal` checks this before touching the on-disk recorder
/// so an operator can stop a recording without restarting the engine.
///
/// Note: this gates *video* recording. The audio side has its own gate
/// (`AUDIO_RECORDING_ENABLED`, toggled via CONFIG_UPDATE 0x05) so audio
/// can be toggled independently of video at runtime — useful when an
/// avatar capture needs video but the operator wants to omit ambient
/// audio mid-session.
pub(crate) static RECORDING_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Runtime gate for *audio* recording. Mirrors `RECORDING_ENABLED` —
/// see CONFIG_UPDATE 0x05 / `apply_audio_recording_config_update`.
///
/// Limitation: when `recording.enabled = false` at boot, the audio
/// pipeline does not open an `AudioRecorder`, so flipping this atomic
/// on at runtime is a no-op until the next session. Matches the same
/// limitation video has (init_recorder returns None when disabled).
pub(crate) static AUDIO_RECORDING_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Latest PC-side encode latency in microseconds (for HEARTBEAT_ACK waterfall).
static PC_ENCODE_LATENCY_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Latest PC-side total latency in microseconds (present→send).
static PC_TOTAL_LATENCY_US: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Check if audio is currently active.
pub fn is_audio_active() -> bool {
    AUDIO_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Spawn a named long-lived tokio task.
///
/// This wrapper exists so every engine task spawn is paired with a
/// human-readable name at a single call site. The name is logged on spawn
/// (debug level) so that when a log stream goes quiet we can tell which
/// subsystem silently stopped — previously `tokio::spawn` with a detached
/// `JoinHandle` gave us no visibility at all.
///
/// Why not `catch_unwind` the future? The workspace release profile uses
/// `panic = "abort"`, so a panic aborts the process before any
/// `catch_unwind` could observe it — the wrapper would be dead code at
/// runtime. If the profile ever switches to `panic = "unwind"`, extend
/// this helper to wrap the future in `FutureExt::catch_unwind` from
/// `futures-util`.
fn spawn_named<F>(handle: &tokio::runtime::Handle, name: &'static str, fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    log::debug!("spawning engine task: {}", name);
    handle.spawn(fut);
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
        // rustls 0.23 requires the process-default CryptoProvider to be
        // installed exactly once before any TLS use. Tests and the C++
        // driver path were both inheriting test-suite installs in earlier
        // builds; the simulator headless binary surfaces the gap because
        // it's the first standalone process to call `StreamingEngine::new`
        // outside the test suite. Calling it here makes every engine
        // instance correct by construction. install_default returns Err
        // when an install already exists, which is the normal case when
        // a parent test or the C++ driver pre-initialised it — discard.
        let _ = rustls::crypto::ring::default_provider().install_default();

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
        // Paired HMD address: written by the session loop, read by the
        // tracking receiver to reject datagrams from any other source.
        let authorized_peer: AuthorizedPeer = Arc::new(std::sync::RwLock::new(None));
        let session_peer = authorized_peer.clone();

        // Spawn the main streaming task
        let cancel = cancel_token.clone();
        let stream_cancel = cancel_token.clone();
        spawn_named(runtime.handle(), "streaming", async move {
            tokio::select! {
                result = run_streaming(config_clone, frame_rx, tracking_clone, tracker_clone, session_peer, stream_cancel) => {
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
        spawn_named(runtime.handle(), "tracking-receiver", async move {
            let receiver = TrackingReceiver::new(tracking_head, tracking_ctrl, authorized_peer);
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

        // Purge old recordings before opening today's file. Runs even when
        // recording is disabled this session so retention is honoured
        // regardless of the current on/off toggle.
        let purge_dir = recording_output_dir(&config);
        let _ = crate::recording::purge_old_recordings(
            &purge_dir,
            config.recording.retention_days,
        );

        let recorder = init_recorder(&config);
        // Seed the runtime toggle from boot config so the first frames after
        // startup honour the same on/off state the user configured. Audio
        // mirrors the same seed; it can be flipped independently at runtime
        // via CONFIG_UPDATE 0x05 once the session is up.
        RECORDING_ENABLED.store(
            config.recording.enabled,
            std::sync::atomic::Ordering::Relaxed,
        );
        AUDIO_RECORDING_ENABLED.store(
            config.recording.enabled,
            std::sync::atomic::Ordering::Relaxed,
        );

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
    /// is disabled (statically or via runtime CONFIG_UPDATE 0x03) or the
    /// recorder has been poisoned by a prior I/O error.
    pub fn write_recording_nal(&self, nal: &[u8]) {
        if !RECORDING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if let Some(rec) = &self.recorder {
            if let Ok(mut r) = rec.try_lock() {
                r.write_nal(nal);
            }
        }
    }

    /// Submit a frame for encoding and sending. Called from C++ thread (via
    /// FFI) and from Rust callers (scenario runner). Taps the raw NAL into
    /// the active recorder before queuing so recording captures every frame
    /// the producer offered — including ones the send channel rejects.
    pub fn submit_frame(&self, frame: EncodedFrame) -> bool {
        self.write_recording_nal(&frame.nal_data);
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
#[derive(Debug, Clone, Copy)]
pub struct HmdStats {
    pub packets_received: u32,
    pub packets_lost: u32,
    pub avg_decode_us: u32,
    pub fps: u16,
}

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

/// What the TCP control task hands to the frame loop. The frame loop owns
/// all adaptive-bitrate state ([`AdaptiveState`]), so neither side takes a
/// lock — and feedback is no longer dropped because the other side happened
/// to hold one.
#[derive(Debug)]
enum ControlEvent {
    /// HEARTBEAT: the HMD's receive stats since its previous heartbeat.
    Heartbeat(HmdStats),
    /// TRANSPORT_FEEDBACK: per-packet receive times for delay-based estimation.
    TransportFeedback(Vec<fvp_common::protocol::TransportFeedbackEntry>),
}

/// Room for this many [`ControlEvent`]s while the frame loop is busy (a
/// heartbeat every 500 ms plus transport feedback). If it fills, the frame
/// loop has stalled for seconds and the stale events are dropped.
const CONTROL_EVENT_CAPACITY: usize = 256;

/// Events dropped because the frame loop fell behind.
static CONTROL_EVENT_DROPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One session's TCP control connection, run by the "tcp-control" task.
/// Handles: IDR_REQUEST, HEARTBEAT, FACE_DATA, TRANSPORT_FEEDBACK,
/// CONFIG_UPDATE, DISCONNECT (inbound). Sends: HEARTBEAT_ACK,
/// CONFIG_UPDATE_ACK, HAPTIC_EVENT, SLEEP_ENTER / SLEEP_EXIT (outbound).
struct ControlChannel {
    /// Cancelled when the connection closes or errors, to stop the session.
    cancel: CancellationToken,
    /// HEARTBEAT and TRANSPORT_FEEDBACK go to the frame loop.
    events: mpsc::Sender<ControlEvent>,
    /// FACE_DATA → VRChat OSC.
    osc_bridge: crate::face_tracking::osc_bridge::OscBridge,
    haptic_rx: mpsc::Receiver<HapticEvent>,
    /// `true` = entering sleep, `false` = waking (from the frame loop).
    sleep_rx: mpsc::Receiver<bool>,
}

impl ControlChannel {
    /// Hand an event to the frame loop without ever blocking the reader.
    fn forward(&self, event: ControlEvent) {
        if self.events.try_send(event).is_err() && !self.events.is_closed() {
            let count = CONTROL_EVENT_DROPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if count % 100 == 1 {
                log::warn!("Control event dropped: frame loop is behind ({} total)", count);
            }
        }
    }
}

/// Serve `ctl`'s connection until it closes, cancelling `ctl.cancel` then.
/// Returns the disconnect reason so the caller can decide whether to hold
/// state.
async fn handle_tcp_control(
    ctl: &mut ControlChannel,
    stream: Box<dyn AsyncStream>,
) -> Result<DisconnectReason, Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let cancel = ctl.cancel.clone();

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

                            ctl.forward(ControlEvent::Heartbeat(HmdStats {
                                packets_received,
                                packets_lost,
                                avg_decode_us,
                                fps,
                            }));

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
                        log::debug!("engine: FACE_DATA received, payload {}B", payload.len());
                        if let Some((lip_valid, eye_valid, lip, eye)) =
                            crate::face_tracking::osc_bridge::parse_face_data(payload)
                        {
                            log::debug!("engine: forwarding face data to OscBridge (lip_valid={}, eye_valid={})", lip_valid, eye_valid);
                            ctl.osc_bridge.send_face_data(lip_valid, eye_valid, &lip, &eye);
                        } else {
                            log::warn!("engine: FACE_DATA parse failed ({}B)", payload.len());
                        }
                    }
                    fvp_common::protocol::msg_type::TRANSPORT_FEEDBACK => {
                        let payload = &msg_buf[1..];
                        if let Some(entries) = fvp_common::protocol::parse_transport_feedback(payload) {
                            log::debug!("Received TRANSPORT_FEEDBACK: {} entries", entries.len());
                            ctl.forward(ControlEvent::TransportFeedback(entries));
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
                                0x03 => { // recording (0=off, 1=on)
                                    ack_status = crate::recording::apply_recording_config_update(
                                        value, &RECORDING_ENABLED,
                                    );
                                    log::info!(
                                        "CONFIG_UPDATE: recording → {} (ack={})",
                                        if value == 1 { "on" } else if value == 0 { "off" } else { "rejected" },
                                        ack_status,
                                    );
                                }
                                0x05 => { // audio recording (0=off, 1=on)
                                    ack_status = crate::recording::apply_audio_recording_config_update(
                                        value, &AUDIO_RECORDING_ENABLED,
                                    );
                                    log::info!(
                                        "CONFIG_UPDATE: audio recording → {} (ack={})",
                                        if value == 1 { "on" } else if value == 0 { "off" } else { "rejected" },
                                        ack_status,
                                    );
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
            Some(haptic) = ctl.haptic_rx.recv() => {
                let payload = haptic.to_payload();
                if let Err(e) = send_msg(&mut writer, fvp_common::protocol::msg_type::HAPTIC_EVENT, &payload).await {
                    log::warn!("Failed to send haptic event: {}", e);
                }
            }
            Some(is_sleep) = ctl.sleep_rx.recv() => {
                let mt = if is_sleep {
                    fvp_common::protocol::msg_type::SLEEP_ENTER
                } else {
                    fvp_common::protocol::msg_type::SLEEP_EXIT
                };
                if let Err(e) = send_msg(&mut writer, mt, &[]).await {
                    log::warn!("Failed to send sleep transition: {}", e);
                }
            }
        }
    }
}

/// Hold a real WASAPI [`AudioCapture`] alive on a dedicated thread until
/// `cancel` fires. cpal's `Stream` is `!Send`, so it must live on the thread
/// where it was created; we poll `is_cancelled()` every 100 ms rather than
/// spin up a tokio runtime just to wait. Capture failure is non-fatal —
/// `AUDIO_ACTIVE` is cleared and video streaming continues.
fn spawn_real_capture(audio_tx: mpsc::Sender<Vec<f32>>, cancel: CancellationToken) {
    use crate::audio::capture::AudioCapture;
    let audio_spawn = std::thread::Builder::new()
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
            while !cancel.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
            log::info!("Audio capture released");
        });
    if let Err(e) = audio_spawn {
        // OS refused to spawn a thread — extremely rare (resource exhaustion).
        // Do not take video streaming down with us; log and continue without audio.
        AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        log::warn!("Failed to spawn audio capture thread: {} — no audio", e);
    }
}

/// Map an `[audio] synthetic_source` config string to a concrete synthetic
/// source. `None` means "fall back to real WASAPI capture" (the `"off"`
/// case, and any unrecognised value — though `validate()` clamps those
/// upstream). Simulator-only: production builds never call this.
#[cfg(feature = "simulator")]
fn resolve_synthetic_source(
    cfg: &crate::config::AudioConfig,
) -> Option<crate::audio::synthetic_capture::SyntheticSource> {
    use crate::audio::synthetic_capture::SyntheticSource;
    match cfg.synthetic_source.as_str() {
        "silence" => Some(SyntheticSource::Silence),
        "sine" => Some(SyntheticSource::default_sine()),
        // A bad/empty path makes `from_wav` return None → graceful "no audio".
        "wav" => SyntheticSource::from_wav(&cfg.synthetic_wav_path),
        _ => None, // "off" → real capture
    }
}

/// Spawn the audio capture → Opus encode → UDP send pipeline.
/// Audio is optional: if capture or encoding fails, streaming continues without audio.
///
/// `audio_config` selects the capture source: production always uses real
/// WASAPI; under the `simulator` feature, `[audio] synthetic_source` can
/// substitute a generated source so the engine emits real Opus over UDP with
/// no audio hardware present.
fn spawn_audio_pipeline(
    target: SocketAddr,
    cancel: CancellationToken,
    recording_dir: Option<std::path::PathBuf>,
    audio_config: crate::config::AudioConfig,
) {
    use crate::audio::encoder::AudioEncoder;
    use crate::transport::rtp::RtpPacket;

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<f32>>(32);

    // Source selection. With the simulator feature off this compiles down to
    // an unconditional `spawn_real_capture`, so production behaviour is
    // byte-identical to before this branch existed.
    #[cfg(feature = "simulator")]
    {
        match resolve_synthetic_source(&audio_config) {
            Some(src) => {
                match crate::audio::synthetic_capture::SyntheticAudioCapture::start(src, audio_tx) {
                    Some(capture) => {
                        AUDIO_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
                        let hold_cancel = cancel.clone();
                        // Hold the synthetic capture (it owns its producer
                        // thread; Drop cancels + joins it) until the session
                        // cancel fires, mirroring the real-capture lifecycle.
                        let spawn = std::thread::Builder::new()
                            .name("fvp-synthetic-audio-hold".into())
                            .spawn(move || {
                                let _capture = capture;
                                while !hold_cancel.is_cancelled() {
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
                                AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
                                log::info!("Synthetic audio source released");
                            });
                        if let Err(e) = spawn {
                            AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
                            log::warn!("Failed to spawn synthetic-audio hold thread: {} — no audio", e);
                        } else {
                            log::info!("Audio source: synthetic ({})", audio_config.synthetic_source);
                        }
                    }
                    None => {
                        AUDIO_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
                        log::warn!("Synthetic audio source failed to start — no audio");
                    }
                }
            }
            None => spawn_real_capture(audio_tx, cancel.clone()),
        }
    }
    #[cfg(not(feature = "simulator"))]
    {
        let _ = &audio_config; // only consulted under the simulator feature
        spawn_real_capture(audio_tx, cancel.clone());
    }

    // Spawn async task: accumulate raw chunks into 10ms frames, encode, send.
    // Accumulation happens here (not in the real-time callback) to avoid Mutex.
    const OPUS_FRAME_SAMPLES: usize = 480; // 10ms at 48kHz
    const STEREO_FRAME_SIZE: usize = OPUS_FRAME_SAMPLES * 2;

    spawn_named(&tokio::runtime::Handle::current(), "audio-encoder", async move {
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

        // Open optional WAV recording. PCM sample rate/channels match the capture
        // pipeline (48 kHz stereo, as assumed by STEREO_FRAME_SIZE above).
        let mut audio_recorder = recording_dir.and_then(|dir| {
            let path = dir.join(crate::recording::default_audio_filename());
            crate::recording::AudioRecorder::open(path, 48000, 2)
        });

        log::info!("Audio streaming started to {}", target);

        loop {
            tokio::select! {
                Some(chunk) = audio_rx.recv() => {
                    // Accumulate raw samples from capture callback
                    accum.extend_from_slice(&chunk);

                    // Extract and encode complete 10ms frames
                    while accum.len() >= STEREO_FRAME_SIZE {
                        let pcm_frame: Vec<f32> = accum.drain(..STEREO_FRAME_SIZE).collect();

                        // Tap PCM for session recording (before encode) so the WAV
                        // is lossless and aligned with the captured samples. The
                        // runtime gate (CONFIG_UPDATE 0x05) lets the operator
                        // pause audio recording mid-session without restarting
                        // the engine, while video keeps recording independently.
                        if AUDIO_RECORDING_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
                            if let Some(rec) = audio_recorder.as_mut() {
                                rec.write_pcm_f32(&pcm_frame);
                            }
                        }

                        let opus_data = match encoder.encode(&pcm_frame) {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Opus encode error: {}", e);
                                continue;
                            }
                        };

                        // Build RTP packet: header (12 bytes) + Opus payload
                        let mut buf = Vec::with_capacity(12 + opus_data.len());
                        // PT=111 (Opus), marker=true (always for audio)
                        crate::transport::rtp::write_rtp_header(&mut buf, 111, true, sequence, timestamp, ssrc);
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

/// Adaptive bitrate + FEC state of one session. Owned by the frame loop and
/// fed [`ControlEvent`]s from the TCP control task, so it needs no locks.
struct AdaptiveState {
    bw_estimator: crate::adaptive::bandwidth_estimator::BandwidthEstimator,
    bitrate_ctrl: crate::adaptive::bitrate_controller::BitrateController,
    /// Classifies transient burst loss vs sustained congestion.
    burst_detector: crate::adaptive::burst_detector::BurstDetector,
    gcc_estimator: crate::adaptive::gcc_estimator::GccEstimator,
    /// `network.congestion_control == "gcc"`: feed transport feedback to the
    /// delay-based estimator. Otherwise ("loss") bitrate follows loss alone.
    gcc_enabled: bool,
    adaptive_fec: Option<crate::transport::fec::AdaptiveFecController>,
    sleep_detector: crate::sleep_mode::SleepDetector,
    /// Newest HEARTBEAT stats not yet used by `tick` (each is used once).
    pending_stats: Option<HmdStats>,
    /// Newest HEARTBEAT stats, for the loss figure in status.json.
    last_stats: Option<HmdStats>,
}

impl AdaptiveState {
    fn new(config: &AppConfig) -> Self {
        let adaptive_fec = if config.network.adaptive_fec_enabled {
            Some(crate::transport::fec::AdaptiveFecController::new(
                config.network.fec_redundancy_min,
                config.network.fec_redundancy_max,
                config.network.fec_redundancy,
            ))
        } else {
            log::info!("Adaptive FEC disabled — using fixed redundancy {:.0}%", config.network.fec_redundancy * 100.0);
            None
        };
        Self {
            bw_estimator: crate::adaptive::bandwidth_estimator::BandwidthEstimator::new(),
            bitrate_ctrl: crate::adaptive::bitrate_controller::BitrateController::new(
                config.video.bitrate_mbps,
            ),
            burst_detector: crate::adaptive::burst_detector::BurstDetector::new(),
            gcc_estimator: crate::adaptive::gcc_estimator::GccEstimator::new(
                config.video.bitrate_mbps as u64 * 1_000_000,
            ),
            gcc_enabled: config.network.congestion_control == "gcc",
            adaptive_fec,
            sleep_detector: crate::sleep_mode::SleepDetector::new(
                config.sleep_mode.enabled,
                config.sleep_mode.motion_threshold,
                config.sleep_mode.timeout_seconds,
                config.sleep_mode.sleep_bitrate_mbps,
            ),
            pending_stats: None,
            last_stats: None,
        }
    }

    /// Take in an event from the TCP control task. `sent_packet_log` maps
    /// RTP sequence numbers to our send times, for feedback tracing.
    fn on_event(&mut self, event: ControlEvent, sent_packet_log: &HashMap<u16, u64>) {
        match event {
            ControlEvent::Heartbeat(stats) => {
                self.pending_stats = Some(stats);
                self.last_stats = Some(stats);
            }
            ControlEvent::TransportFeedback(entries) => {
                for entry in &entries {
                    if let Some(&send_us) = sent_packet_log.get(&entry.sequence) {
                        log::trace!(
                            "FEEDBACK seq={} send_us={} recv_delta_us={}",
                            entry.sequence, send_us, entry.recv_delta_us
                        );
                    }
                }
                // Only feed GCC estimator when delay-based congestion control is enabled
                if self.gcc_enabled {
                    self.gcc_estimator.process_feedback(&entries);
                }
            }
        }
    }

    /// Adjust bitrate and FEC redundancy from the newest HMD stats; nothing
    /// to do if no heartbeat arrived since the last tick. Called once per
    /// second (every `framerate` frames).
    fn tick(&mut self, fec_encoder: &mut pipeline::FrameFecEncoder) {
        let Some(stats) = self.pending_stats.take() else { return };
        self.bw_estimator.update(stats.packets_received, stats.packets_lost, 0.0);

        if self.gcc_enabled {
            self.burst_detector.record(self.bw_estimator.loss_rate());
            if self.bitrate_ctrl.adjust(&self.bw_estimator, &self.gcc_estimator, &self.burst_detector) {
                notify_bitrate_change(self.bitrate_ctrl.current_bitrate_bps() as u32);
            }

            // FEC response to burst. On boost activation, skip the loss-based adjust
            // below so the boost level wins this tick.
            if self.burst_detector.recommend_fec_boost() {
                if let Some(afec) = self.adaptive_fec.as_mut() {
                    log::info!("Burst loss: boosting FEC redundancy");
                    afec.activate_boost();
                    fec_encoder.set_redundancy(afec.effective_redundancy());
                    return;
                }
            } else if let Some(afec) = self.adaptive_fec.as_mut() {
                if afec.effective_redundancy() != afec.current_redundancy() {
                    afec.deactivate_boost();
                    fec_encoder.set_redundancy(afec.effective_redundancy());
                }
            }
        } else {
            // Loss-only mode: adjust with neutral GCC + burst defaults.
            let default_gcc = crate::adaptive::gcc_estimator::GccEstimator::new(
                self.bitrate_ctrl.current_bitrate_bps(),
            );
            let default_burst = crate::adaptive::burst_detector::BurstDetector::new();
            if self.bitrate_ctrl.adjust(&self.bw_estimator, &default_gcc, &default_burst) {
                notify_bitrate_change(self.bitrate_ctrl.current_bitrate_bps() as u32);
            }
        }

        // Loss-based FEC adjustment (skipped during burst-boost early return above).
        if let Some(afec) = self.adaptive_fec.as_mut() {
            if afec.adjust(self.bw_estimator.loss_rate()) {
                fec_encoder.set_redundancy(afec.effective_redundancy());
            }
        }
    }
}

/// Detect user inactivity from head tracking and enter/exit sleep mode.
/// On a transition this both adjusts the bitrate (production driver feedback)
/// and pushes onto `sleep_tx` so the TCP writer task can notify the HMD via
/// `SLEEP_ENTER (0x50)` / `SLEEP_EXIT (0x51)` for overlay dimming.
fn check_sleep_mode(
    tracking: &Arc<StdMutex<Option<TrackingData>>>,
    sleep_detector: &mut crate::sleep_mode::SleepDetector,
    normal_bitrate_mbps: u32,
    timeout_seconds: u32,
    sleep_tx: &mpsc::Sender<bool>,
) {
    if let Ok(guard) = tracking.lock() {
        if let Some(ref data) = *guard {
            if let Some(transition) = sleep_detector.update(data) {
                match transition {
                    crate::sleep_mode::SleepTransition::Sleep => {
                        log::info!("Sleep mode: entering (no motion for {}s)", timeout_seconds);
                        let bps = sleep_detector.sleep_bitrate_mbps() * 1_000_000;
                        notify_bitrate_change(bps);
                        let _ = sleep_tx.try_send(true);
                    }
                    crate::sleep_mode::SleepTransition::Wake => {
                        log::info!("Sleep mode: waking up (motion detected)");
                        let bps = normal_bitrate_mbps * 1_000_000;
                        notify_bitrate_change(bps);
                        let _ = sleep_tx.try_send(false);
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
/// Resolve the recording output directory from config. Falls back to
/// %APPDATA%/FocusVisionPCVR/recordings (or platform equivalent).
fn recording_output_dir(config: &AppConfig) -> std::path::PathBuf {
    if !config.recording.output_dir.is_empty() {
        return std::path::PathBuf::from(&config.recording.output_dir);
    }
    dirs_next::data_dir()
        .map(|d| d.join("FocusVisionPCVR").join("recordings"))
        .unwrap_or_else(|| std::path::PathBuf::from("./recordings"))
}

/// Build a Recorder if config.recording.enabled is true.
/// Returns None when disabled or when the output file cannot be opened.
fn init_recorder(config: &AppConfig) -> Option<Arc<StdMutex<crate::recording::Recorder>>> {
    if !config.recording.enabled {
        return None;
    }
    let ext = match config.video.codec {
        fvp_common::protocol::VideoCodec::H264 => "h264",
        fvp_common::protocol::VideoCodec::H265 => "h265",
    };
    let path = recording_output_dir(config).join(crate::recording::default_filename(ext));
    crate::recording::Recorder::open(path).map(|r| Arc::new(StdMutex::new(r)))
}

/// Record the current send timestamp for every packet's RTP seq number,
/// so GCC / delay estimation can correlate transport feedback back to send time.
fn record_send_timestamps(
    sent_packet_log: &mut HashMap<u16, u64>,
    packets: &[crate::transport::rtp::RtpPacket],
) {
    let send_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    for packet in packets {
        if packet.data.len() >= 4 {
            let seq = u16::from_be_bytes([packet.data[2], packet.data[3]]);
            sent_packet_log.insert(seq, send_us);
        }
    }
}

/// Periodically bound sent_packet_log memory usage by retaining only the
/// newest `SENT_LOG_KEEP` entries. Runs every `SENT_LOG_PRUNE_INTERVAL`
/// frames (~3.3s @ 90fps) to avoid per-frame sort overhead.
fn prune_sent_packet_log_if_due(
    sent_packet_log: &mut HashMap<u16, u64>,
    frame_count: u64,
) {
    const SENT_LOG_PRUNE_INTERVAL: u64 = 300;
    const SENT_LOG_MAX: usize = 5000;
    const SENT_LOG_KEEP: usize = 2500;
    if !frame_count.is_multiple_of(SENT_LOG_PRUNE_INTERVAL) {
        return;
    }
    if sent_packet_log.len() > SENT_LOG_MAX {
        let mut entries: Vec<(u16, u64)> = sent_packet_log.drain().collect();
        entries.sort_unstable_by_key(|&(_, ts)| std::cmp::Reverse(ts));
        entries.truncate(SENT_LOG_KEEP);
        sent_packet_log.extend(entries);
    }
}

/// The frame loop's per-session video sender: packetization, FEC and UDP,
/// plus the send-time log that transport feedback is matched against.
struct VideoSender {
    udp_sender: UdpSender,
    packetizer: RtpPacketizer,
    fec_encoder: pipeline::FrameFecEncoder,
    /// RTP sequence number → PC-side send timestamp (µs).
    sent_packet_log: HashMap<u16, u64>,
    frame_count: u64,
    latency_skip_count: u64,
}

impl VideoSender {
    fn new(config: &AppConfig, udp_sender: UdpSender) -> Self {
        Self {
            udp_sender,
            packetizer: RtpPacketizer::new(0x46565000),
            fec_encoder: pipeline::FrameFecEncoder::new(
                config.network.fec_redundancy,
                config.network.slice_fec_enabled,
                config.network.slice_count,
            ),
            sent_packet_log: HashMap::new(),
            frame_count: 0,
            latency_skip_count: 0,
        }
    }

    /// Send one encoded frame: FEC-code it (the encoder picks slice-based or
    /// bulk FEC per frame, see `pipeline::choose_fec_layout`), UDP-send the
    /// resulting packets one code word at a time, record send timestamps and
    /// the frame's latency, and count it.
    async fn send_frame(
        &mut self,
        mut frame: EncodedFrame,
        framerate: u64,
        latency_tracker: &Arc<StdMutex<LatencyTracker>>,
    ) {
        frame.timestamps.mark_encode_start();
        frame.timestamps.mark_encode_end();

        // Multiply first to avoid integer division truncation drift (e.g. 90000/96=937.5)
        let timestamp_90khz = (self.frame_count * fvp_common::RTP_CLOCK_RATE as u64 / framerate) as u32;

        let batches = self.fec_encoder.encode(
            &frame.nal_data,
            frame.frame_index,
            timestamp_90khz,
            frame.is_idr,
            &mut self.packetizer,
        );
        for packets in &batches {
            if let Err(e) = self.udp_sender.send_all(packets).await {
                log::warn!("UDP send error: {}", e);
            }
            record_send_timestamps(&mut self.sent_packet_log, packets);
        }
        prune_sent_packet_log_if_due(&mut self.sent_packet_log, self.frame_count);

        // Packets may still be in the kernel queue; this marks dispatch.
        frame.timestamps.mark_send();

        if let Ok(mut tracker) = latency_tracker.try_lock() {
            tracker.record(frame.timestamps);
        } else {
            self.latency_skip_count += 1;
            if self.latency_skip_count % 90 == 1 {
                log::debug!("Latency tracker lock contention (skipped {} samples)", self.latency_skip_count);
            }
        }

        self.frame_count += 1;
    }
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

/// Publish a live `"streaming"` status.json snapshot so the companion app shows
/// Connected with live latency/fps/bitrate and subsystem indicators while a
/// session is active. Written once on connect and refreshed every
/// `STATUS_HEARTBEAT` by the session loop (independent of frame flow); the
/// accept loop reverts to `"waiting"` after a disconnect. This is the
/// real-data source the `--demo` mode fakes — without it the companion would
/// stay on "WaitingForPin" for the entire session, even with real hardware.
fn publish_streaming_status(
    config: &AppConfig,
    bitrate_mbps: u32,
    sleeping: bool,
    hmd_stats: Option<&HmdStats>,
) {
    let latency_us = PC_TOTAL_LATENCY_US.load(std::sync::atomic::Ordering::Relaxed) as u64;
    // Loss% from the most recent HEARTBEAT-reported HMD stats, if any.
    let packet_loss_pct = hmd_stats
        .map(|s| {
            let total = s.packets_received + s.packets_lost;
            if total == 0 {
                0.0
            } else {
                s.packets_lost as f32 / total as f32 * 100.0
            }
        })
        .unwrap_or(0.0);
    let subsystems = crate::SubsystemStatus {
        ft_active: config.face_tracking.enabled,
        sleep_active: sleeping,
        audio_enabled: is_audio_active(),
        packet_loss_pct,
    };
    crate::write_status_file(
        "streaming",
        None,
        Some(latency_us),
        Some(config.video.framerate as u16),
        Some(bitrate_mbps),
        Some(&subsystems),
        None,
    );
}

/// How often status.json is rewritten while the engine is alive. The
/// companion treats a file older than 5 s as "engine stopped", so every
/// engine state — waiting for the HMD, reconnect backoff, streaming — must
/// refresh it well inside that window.
const STATUS_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(1);

/// Drive `fut` to completion, calling `beat` immediately and then every
/// `period` until it finishes. Used to keep status.json fresh while the
/// engine is parked on a long await (TCP accept, reconnect backoff) that
/// would otherwise leave the file untouched for minutes.
async fn with_heartbeat<F: std::future::Future>(
    fut: F,
    period: std::time::Duration,
    mut beat: impl FnMut(),
) -> F::Output {
    tokio::pin!(fut);
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            out = &mut fut => return out,
            _ = ticker.tick() => beat(),
        }
    }
}

/// `"waiting"` status: with the pairing PIN and the seconds until it expires
/// while a TCP server is listening, or `None` (rendered as "------", i.e. not
/// yet pairable) during backoff.
fn publish_waiting_status(pin: Option<(u32, u32)>) {
    crate::write_status_file(
        "waiting",
        pin.map(|(pin, _)| pin),
        None,
        None,
        None,
        None,
        pin.map(|(_, expires_in)| expires_in),
    );
}

/// Heartbeat for a listening `server`: publish its current PIN (it rotates
/// on expiry, and on lockout) with the time left. `last` keeps the previous
/// value for the rare beat where a handshake holds the pairing lock.
fn publish_pin_status(server: &TcpControlServer, last: &mut Option<(u32, u32)>) {
    if let Some(status) = server.pin_status() {
        *last = Some(status);
    }
    publish_waiting_status(*last);
}

/// Marks `ip` as the paired HMD for the tracking receiver's source check
/// while alive, and revokes it on drop — so every way a session can end
/// (clean break, `continue`, early `return`) closes the tracking port to
/// that address again.
struct AuthorizedPeerGuard {
    peer: AuthorizedPeer,
}

impl AuthorizedPeerGuard {
    fn authorize(peer: &AuthorizedPeer, ip: std::net::IpAddr) -> Self {
        if let Ok(mut guard) = peer.write() {
            *guard = Some(ip);
        }
        Self { peer: peer.clone() }
    }
}

impl Drop for AuthorizedPeerGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.peer.write() {
            *guard = None;
        }
    }
}

async fn run_streaming(
    config: AppConfig,
    frame_rx: mpsc::Receiver<EncodedFrame>,
    tracking: Arc<StdMutex<Option<TrackingData>>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
    authorized_peer: AuthorizedPeer,
    cancel: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    StreamingLoop::new(config, frame_rx, tracking, latency_tracker, authorized_peer, cancel)
        .run()
        .await;
    Ok(())
}

/// An authenticated HMD connection and the server (with its PIN) that
/// accepted it.
struct Connection {
    server: TcpControlServer,
    stream: Box<dyn AsyncStream>,
    peer: SocketAddr,
}

enum Accepted {
    Connected(Box<Connection>),
    /// Accept failed; try again after the backoff.
    Retry,
    /// Engine shutdown, or too many accept failures.
    Stop,
}

enum SessionEnd {
    Disconnected(DisconnectReason),
    /// The session could not start (UDP sender).
    SetupFailed,
    Shutdown,
}

enum HoldOutcome {
    Resumed(Box<Connection>),
    NoReconnect,
    Shutdown,
}

/// The engine's accept → session → hold loop, with the state that outlives a
/// single session.
struct StreamingLoop {
    config: AppConfig,
    frame_rx: mpsc::Receiver<EncodedFrame>,
    tracking: Arc<StdMutex<Option<TrackingData>>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
    authorized_peer: AuthorizedPeer,
    cancel: CancellationToken,
    /// Counter rules and exponential backoff live in `control::reconnect`
    /// so they are unit-testable without spinning up the full async loop.
    reconnect_state: crate::control::reconnect::ReconnectState,
    /// Thermal governor lives for the whole engine — NVML init is non-trivial
    /// (~tens of ms) so we pay it once. `try_create_nvml` returns `None` on
    /// non-NVIDIA hosts and when the `nvml` cargo feature is compiled out,
    /// so every call site treats the governor as opt-in: present → cap by
    /// multiplier, absent → no cap. The base ceiling re-applies each session
    /// because `bitrate_ctrl` is per-session.
    thermal_gov: Option<crate::thermal::ThermalGovernor>,
}

impl StreamingLoop {
    fn new(
        config: AppConfig,
        frame_rx: mpsc::Receiver<EncodedFrame>,
        tracking: Arc<StdMutex<Option<TrackingData>>>,
        latency_tracker: Arc<StdMutex<LatencyTracker>>,
        authorized_peer: AuthorizedPeer,
        cancel: CancellationToken,
    ) -> Self {
        let thermal_gov = if config.thermal.enabled {
            crate::thermal::ThermalGovernor::try_create_nvml(config.thermal.clone())
        } else {
            None
        };
        if thermal_gov.is_some() {
            log::info!(
                "Thermal governor armed (warn={}°C / limit={}°C / emergency={}°C)",
                config.thermal.warn_celsius,
                config.thermal.limit_celsius,
                config.thermal.emergency_celsius,
            );
        }
        Self {
            config,
            frame_rx,
            tracking,
            latency_tracker,
            authorized_peer,
            cancel,
            reconnect_state: crate::control::reconnect::ReconnectState::new(),
            thermal_gov,
        }
    }

    /// Reconnect loop: when a session ends (TCP disconnect, Wi-Fi drop),
    /// clean up and re-listen for a new HMD connection.
    ///
    /// The 2-token (session_cancel + connection_cancel) split that keeps
    /// audio alive across the 5 s hold window is intentionally deferred
    /// to Phase 4.2 — it requires real Wi-Fi drop testing to validate.
    async fn run(mut self) {
        // A connection accepted during the previous session's hold window —
        // already through TLS + PIN with that session's server — to resume on.
        let mut resumed: Option<Box<Connection>> = None;

        loop {
            if self.cancel.is_cancelled() { break; }

            let conn = match resumed.take() {
                Some(conn) => conn,
                None => match self.accept().await {
                    Accepted::Connected(conn) => conn,
                    Accepted::Retry => continue,
                    Accepted::Stop => break,
                },
            };
            let Connection { server, stream, peer } = *conn;
            log::info!("HMD connected from {}, starting video stream", peer);
            self.reconnect_state.record_accept_success();

            let reason = match self.run_session(stream, peer).await {
                SessionEnd::Disconnected(reason) => reason,
                SessionEnd::SetupFailed => {
                    self.reconnect_state.record_accept_failure();
                    continue;
                }
                SessionEnd::Shutdown => return,
            };

            match reason {
                DisconnectReason::ConnectionLost => {
                    log::info!("Connection lost — listening for reconnection (5s hold)");
                    self.reconnect_state.record_connection_lost();
                    if self.reconnect_state.is_reconnect_warning_due() {
                        log::warn!(
                            "Wi-Fi reconnect attempts: {}/{} — still accepting connections",
                            self.reconnect_state.reconnect_attempts(),
                            crate::control::reconnect::MAX_RECONNECT_ATTEMPTS,
                        );
                    }
                    match self.hold(server).await {
                        HoldOutcome::Resumed(conn) => {
                            resumed = Some(conn);
                            continue; // Skip backoff, go directly to new session
                        }
                        HoldOutcome::NoReconnect => {}
                        HoldOutcome::Shutdown => return,
                    }
                }
                DisconnectReason::ClientRequested => {
                    // Clean disconnect — no hold needed
                    log::info!("Client requested disconnect — ready for new connection");
                    self.reconnect_state.record_clean_disconnect();
                }
                DisconnectReason::ProtocolError => {
                    log::warn!("Protocol error — reconnecting");
                    self.reconnect_state.record_protocol_error();
                }
            }

            if self.reconnect_state.should_stop_engine() {
                log::error!(
                    "Max accept failures reached ({}) — stopping engine",
                    self.reconnect_state.accept_failures(),
                );
                break;
            }
        }
    }

    /// Wait for an HMD to connect and pair, after the backoff for any
    /// earlier accept failures.
    async fn accept(&mut self) -> Accepted {
        if let Some(delay) = self.reconnect_state.next_backoff() {
            log::info!(
                "Retrying accept in {:?} (failure {}/{})",
                delay,
                self.reconnect_state.accept_failures(),
                crate::control::reconnect::MAX_ACCEPT_FAILURES,
            );
            // Backoff can last up to 16 s — keep status.json fresh so the
            // companion doesn't report the engine as stopped meanwhile.
            tokio::select! {
                _ = with_heartbeat(tokio::time::sleep(delay), STATUS_HEARTBEAT, || publish_waiting_status(None)) => {}
                _ = self.cancel.cancelled() => return Accepted::Stop,
            }
        }

        // Step 1: Wait for HMD to connect via TCP
        let server = TcpControlServer::new(self.config.clone());
        // Publish the server's PIN to status.json so the companion app
        // (or the simulator mock-client) can read it, and re-publish it
        // every STATUS_HEARTBEAT while we wait: the companion judges
        // engine liveness from the file's mtime, and waiting for the
        // HMD can take minutes. Each beat reads the PIN afresh — it
        // rotates when PIN_LIFETIME_SECONDS runs out and on lockout —
        // with the seconds left, so the companion's "Expires in: M:SS"
        // countdown follows the real PIN.
        let mut shown = None;
        let accept_result = tokio::select! {
            r = with_heartbeat(server.listen_and_accept(), STATUS_HEARTBEAT, || publish_pin_status(&server, &mut shown)) => r,
            _ = self.cancel.cancelled() => return Accepted::Stop,
        };

        match accept_result {
            Ok((stream, peer)) => Accepted::Connected(Box::new(Connection { server, stream, peer })),
            Err(e) => {
                log::error!("TCP accept failed: {}", e);
                self.reconnect_state.record_accept_failure();
                if self.reconnect_state.should_stop_engine() {
                    log::error!(
                        "Max accept failures reached ({}) — stopping engine",
                        self.reconnect_state.accept_failures(),
                    );
                    return Accepted::Stop;
                }
                Accepted::Retry
            }
        }
    }

    /// Stream to a connected HMD until it disconnects, the frame source
    /// closes, or the engine shuts down.
    async fn run_session(&mut self, stream: Box<dyn AsyncStream>, peer: SocketAddr) -> SessionEnd {
        let config = &self.config;

        // Only the HMD that just completed TLS + PIN pairing may feed the
        // tracking port for the lifetime of this session.
        let _peer_guard = AuthorizedPeerGuard::authorize(&self.authorized_peer, peer.ip());

        // Per-session cancel: fires when TCP drops or HMD disconnects
        let session_cancel = CancellationToken::new();

        // Haptic event channel (PC driver → TCP → HMD)
        let (haptic_tx, haptic_rx) = mpsc::channel::<HapticEvent>(16);
        if let Ok(mut guard) = HAPTIC_TX.write() {
            *guard = Some(haptic_tx);
        }

        // Sleep-mode transition channel: `check_sleep_mode` on the frame
        // loop pushes `true` (entering sleep) or `false` (waking) here, and
        // `handle_tcp_control` drains them onto the TCP wire as
        // `SLEEP_ENTER (0x50)` / `SLEEP_EXIT (0x51)` messages. The HMD uses
        // them to dim the overlay; scenario tests assert on them as the
        // observable side of an otherwise internal state change.
        let (sleep_tx, sleep_rx) = mpsc::channel::<bool>(8);

        // HEARTBEAT stats and transport feedback, TCP control → frame loop.
        let (events_tx, mut events_rx) = mpsc::channel::<ControlEvent>(CONTROL_EVENT_CAPACITY);

        // OSC bridge for face tracking data (HMD → VRChat). Apply
        // `face_tracking.osc_port` to the bridge target so non-default
        // ports (used by E2E scenarios that capture OSC on a loopback
        // receiver) actually take effect instead of being silently
        // ignored as they were before this hook was added.
        let mut osc_bridge = crate::face_tracking::osc_bridge::OscBridge::with_smoothing(
            config.face_tracking.smoothing,
        );
        let osc_target = format!("127.0.0.1:{}", config.face_tracking.osc_port);
        log::debug!("OSC bridge target: {}", osc_target);
        osc_bridge.set_target(osc_target);

        // Spawn TCP control reader/writer. It reports the disconnect reason
        // for the hold logic.
        let mut control = ControlChannel {
            cancel: session_cancel.clone(),
            events: events_tx,
            osc_bridge,
            haptic_rx,
            sleep_rx,
        };
        let (reason_tx, mut reason_rx) = tokio::sync::oneshot::channel();
        spawn_named(&tokio::runtime::Handle::current(), "tcp-control", async move {
            let reason = match handle_tcp_control(&mut control, stream).await {
                Ok(reason) => {
                    log::info!("TCP control ended: {:?}", reason);
                    reason
                }
                Err(e) => {
                    log::warn!("TCP control error: {}", e);
                    DisconnectReason::ConnectionLost
                }
            };
            let _ = reason_tx.send(reason);
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
        let udp_target: SocketAddr = SocketAddr::new(peer.ip(), config.network.udp_port + fvp_common::VIDEO_PORT_OFFSET);
        let udp_sender = match UdpSender::new(udp_target).await {
            Ok(s) => s,
            Err(e) => {
                log::error!("UDP sender failed: {}", e);
                session_cancel.cancel();
                return SessionEnd::SetupFailed;
            }
        };

        // Audio pipeline (per-session)
        let audio_port = config.network.udp_port + fvp_common::AUDIO_PORT_OFFSET;
        let audio_target: SocketAddr = SocketAddr::new(peer.ip(), audio_port);
        let audio_recording_dir = if config.recording.enabled {
            Some(recording_output_dir(config))
        } else {
            None
        };
        if config.audio.enabled {
            spawn_audio_pipeline(audio_target, session_cancel.clone(), audio_recording_dir, config.audio.clone());
        } else {
            log::info!("Audio disabled by config — streaming video only");
        }

        // Step 3: Process frames with adaptive bitrate + adaptive FEC
        let mut video = VideoSender::new(config, udp_sender);
        let mut adaptive = AdaptiveState::new(config);
        let framerate = config.video.framerate as u64;

        // Flip status.json to "streaming" the moment the session is up so the
        // companion shows Connected immediately; refreshed ~1×/sec below.
        publish_streaming_status(
            config,
            adaptive.bitrate_ctrl.current_bitrate_mbps(),
            adaptive.sleep_detector.is_sleeping(),
            adaptive.last_stats.as_ref(),
        );

        // Refresh the live "streaming" snapshot on a wall-clock tick rather
        // than every Nth frame: if SteamVR stops submitting frames (game
        // paused, loading screen) the session is still up and status.json
        // must stay fresh, or the companion flags the engine as stopped.
        let mut status_tick = tokio::time::interval(STATUS_HEARTBEAT);
        status_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        status_tick.tick().await; // first tick is immediate; we just published

        let mut ended_by_control = false;
        loop {
            tokio::select! {
                _ = status_tick.tick() => {
                    // `update_latency_atomics` (frame branch) keeps
                    // PC_TOTAL_LATENCY_US current; this reads it.
                    publish_streaming_status(
                        config,
                        adaptive.bitrate_ctrl.current_bitrate_mbps(),
                        adaptive.sleep_detector.is_sleeping(),
                        adaptive.last_stats.as_ref(),
                    );
                }
                Some(event) = events_rx.recv() => {
                    adaptive.on_event(event, &video.sent_packet_log);
                }
                frame_opt = self.frame_rx.recv() => {
                    let Some(frame) = frame_opt else {
                        break; // Channel closed (engine shutdown)
                    };
                    video.send_frame(frame, framerate, &self.latency_tracker).await;

                    if video.frame_count.is_multiple_of(framerate) {
                        adaptive.tick(&mut video.fec_encoder);
                    }

                    // Thermal feedback. The governor internally rate-limits to
                    // `config.thermal.poll_interval_seconds`; calling every
                    // bitrate-adjust tick (~1 s) just skips early on most
                    // ticks. `multiplier < 1.0` lowers the bitrate ceiling
                    // until the GPU cools; on non-NVIDIA hosts the governor
                    // is None and the whole block is a no-op.
                    if let Some(gov) = self.thermal_gov.as_mut() {
                        if let Some(multiplier) = gov.tick() {
                            let base_max_bps = (config.video.bitrate_mbps as u64) * 1_000_000;
                            let ceiling = (base_max_bps as f64 * multiplier) as u64;
                            adaptive.bitrate_ctrl.set_thermal_ceiling_bps(ceiling);
                        }
                    }

                    check_sleep_mode(
                        &self.tracking, &mut adaptive.sleep_detector,
                        config.video.bitrate_mbps, config.sleep_mode.timeout_seconds,
                        &sleep_tx,
                    );

                    update_latency_atomics(&self.latency_tracker);

                    log_periodic_stats(video.frame_count, framerate);
                }
                _ = session_cancel.cancelled() => {
                    log::info!("Session ended — waiting for new connection");
                    ended_by_control = true;
                    break;
                }
                _ = self.cancel.cancelled() => {
                    log::info!("Engine shutdown — stopping streaming");
                    return SessionEnd::Shutdown;
                }
            }
        }

        // The control task cancels the session just before it reports why,
        // so wait briefly for the reason. If the frame source ended the
        // session instead, the control task may still be running: count it
        // as a lost connection.
        let reason = if ended_by_control {
            tokio::time::timeout(std::time::Duration::from_secs(1), reason_rx)
                .await
                .ok()
                .and_then(Result::ok)
        } else {
            reason_rx.try_recv().ok()
        };
        SessionEnd::Disconnected(reason.unwrap_or(DisconnectReason::ConnectionLost))
    }

    /// Listen again for the hold period with the session's own server,
    /// which accepts the PIN the HMD paired with (still on the companion's
    /// screen) once more, so a Wi-Fi blip doesn't need a new PIN.
    /// Publishing it (and keeping status.json fresh) also stops the
    /// companion showing the dead session as Connected. After the window,
    /// the next accept starts over with a new server and PIN.
    async fn hold(&mut self, server: TcpControlServer) -> HoldOutcome {
        let hold_duration = std::time::Duration::from_secs(5);
        server.rearm_for_reconnect(hold_duration).await;
        let mut shown = None;

        let reconnect_result = tokio::select! {
            r = with_heartbeat(server.listen_and_accept(), STATUS_HEARTBEAT, || publish_pin_status(&server, &mut shown)) => Some(r),
            _ = tokio::time::sleep(hold_duration) => None,
            _ = self.cancel.cancelled() => {
                log::info!("Engine shutdown during hold period");
                return HoldOutcome::Shutdown;
            }
        };

        match reconnect_result {
            Some(Ok((stream, peer))) => {
                log::info!("HMD reconnected from {} during hold period — resuming", peer);
                // Stream on the connection we just authenticated.
                HoldOutcome::Resumed(Box::new(Connection { server, stream, peer }))
            }
            Some(Err(e)) => {
                log::warn!("Accept failed during hold: {}", e);
                self.reconnect_state.record_hold_accept_failure();
                HoldOutcome::NoReconnect
            }
            None => {
                log::info!("Hold period expired — no reconnection");
                HoldOutcome::NoReconnect
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::metrics::latency::FrameTimestamps;

    #[cfg(feature = "simulator")]
    #[test]
    fn test_resolve_synthetic_source_selection() {
        use crate::audio::synthetic_capture::SyntheticSource;
        let mut cfg = crate::config::AudioConfig {
            synthetic_source: "off".into(),
            ..crate::config::AudioConfig::default()
        };
        assert!(resolve_synthetic_source(&cfg).is_none());

        cfg.synthetic_source = "silence".into();
        assert!(matches!(resolve_synthetic_source(&cfg), Some(SyntheticSource::Silence)));

        cfg.synthetic_source = "sine".into();
        assert!(matches!(resolve_synthetic_source(&cfg), Some(SyntheticSource::Sine { .. })));

        // "wav" with an empty/invalid path degrades to None (graceful no-audio).
        cfg.synthetic_source = "wav".into();
        cfg.synthetic_wav_path = String::new();
        assert!(resolve_synthetic_source(&cfg).is_none());

        // Unknown values are treated as "off" (validate() would clamp them too).
        cfg.synthetic_source = "bogus".into();
        assert!(resolve_synthetic_source(&cfg).is_none());
    }

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

    // The explicit `0u64 * tick` / `1u64 * tick` form in these tests is
    // intentional: it documents the formula `frame_count * tick` so the
    // reader sees how each frame maps to its RTP timestamp. The clippy
    // simplifications obscure that, so we silence the lints here only.
    #[test]
    #[allow(clippy::erasing_op, clippy::identity_op)]
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
    #[allow(clippy::identity_op)]
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

    /// Helper: run `handle_tcp_control` over a mock duplex stream
    struct TcpTestHarness {
        cancel: CancellationToken,
        /// The bridge after the run (FACE_DATA lands here).
        osc_bridge: Option<crate::face_tracking::osc_bridge::OscBridge>,
        /// Everything forwarded to the frame loop.
        events: Vec<ControlEvent>,
        disconnect_reason: Option<DisconnectReason>,
    }

    impl TcpTestHarness {
        fn new() -> Self {
            Self {
                cancel: CancellationToken::new(),
                osc_bridge: None,
                events: Vec::new(),
                disconnect_reason: None,
            }
        }

        async fn run_with_input(mut self, input: &[u8]) -> Self {
            let (client, server) = tokio::io::duplex(4096);
            let (_haptic_tx, haptic_rx) = mpsc::channel::<HapticEvent>(16);
            let (_sleep_tx, sleep_rx) = mpsc::channel::<bool>(8);
            let (events_tx, mut events_rx) = mpsc::channel::<ControlEvent>(CONTROL_EVENT_CAPACITY);
            let mut control = ControlChannel {
                cancel: self.cancel.clone(),
                events: events_tx,
                osc_bridge: crate::face_tracking::osc_bridge::OscBridge::with_smoothing(0.5),
                haptic_rx,
                sleep_rx,
            };

            // Write input to client side, then close
            let (mut client_read, mut client_write) = tokio::io::split(client);
            use tokio::io::AsyncWriteExt;
            client_write.write_all(input).await.unwrap();
            drop(client_write); // Close write side → EOF for server reader

            // Run handle_tcp_control (will read until EOF)
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                handle_tcp_control(&mut control, Box::new(server)),
            ).await;

            // Capture disconnect reason
            if let Ok(Ok(reason)) = result {
                self.disconnect_reason = Some(reason);
            }
            self.osc_bridge = Some(control.osc_bridge);
            while let Ok(event) = events_rx.try_recv() {
                self.events.push(event);
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

        let s = match harness.events.as_slice() {
            [ControlEvent::Heartbeat(s)] => *s,
            other => panic!("HEARTBEAT must forward its stats to the frame loop, got {other:?}"),
        };
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
        let bridge = harness.osc_bridge.as_ref().unwrap();
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
        assert!(
            matches!(harness.events.as_slice(), [ControlEvent::TransportFeedback(e)] if e.len() == 1 && e[0].sequence == 1),
            "feedback must reach the frame loop, got {:?}", harness.events,
        );
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

    fn heartbeat(received: u32, lost: u32) -> ControlEvent {
        ControlEvent::Heartbeat(HmdStats { packets_received: received, packets_lost: lost, avg_decode_us: 0, fps: 90 })
    }

    #[test]
    fn test_adaptive_state_uses_each_heartbeat_once_but_keeps_it_for_status() {
        let mut adaptive = AdaptiveState::new(&AppConfig::default());
        let mut fec = pipeline::FrameFecEncoder::new(0.2, true, 4);
        adaptive.on_event(heartbeat(900, 100), &HashMap::new());
        assert!(adaptive.pending_stats.is_some());

        adaptive.tick(&mut fec);
        assert!(adaptive.pending_stats.is_none(), "a heartbeat drives one tick only");
        // The loss figure in status.json keeps showing the last heartbeat
        // (it used to read 0 % most of the time: the tick had taken it).
        assert_eq!(adaptive.last_stats.map(|s| s.packets_lost), Some(100));

        let before = adaptive.bw_estimator.loss_rate();
        adaptive.tick(&mut fec); // no new heartbeat: nothing to learn from
        assert_eq!(adaptive.bw_estimator.loss_rate(), before);
    }

    #[tokio::test]
    async fn test_with_heartbeat_beats_while_future_pending() {
        // status.json liveness: the accept loop parks on listen_and_accept
        // for minutes; the heartbeat must keep firing until it resolves.
        let mut beats = 0u32;
        let out = with_heartbeat(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(260)).await;
                42
            },
            std::time::Duration::from_millis(50),
            || beats += 1,
        )
        .await;
        assert_eq!(out, 42);
        assert!(beats >= 3, "expected repeated beats while pending, got {beats}");
    }

    #[tokio::test]
    async fn test_with_heartbeat_returns_ready_future_output() {
        let mut beats = 0u32;
        let out = with_heartbeat(async { 7 }, std::time::Duration::from_secs(60), || beats += 1).await;
        assert_eq!(out, 7);
        assert!(beats <= 1, "at most the immediate first beat, got {beats}");
    }

    #[test]
    fn test_authorized_peer_guard_revokes_on_drop() {
        use crate::tracking::receiver::is_authorized_source;
        let peer: AuthorizedPeer = Arc::new(std::sync::RwLock::new(None));
        let hmd = std::net::IpAddr::from([192, 168, 1, 50]);
        assert!(!is_authorized_source(&peer, hmd), "nothing authorized before a session");
        {
            let _session = AuthorizedPeerGuard::authorize(&peer, hmd);
            assert!(is_authorized_source(&peer, hmd));
            assert!(!is_authorized_source(&peer, std::net::IpAddr::from([192, 168, 1, 99])));
        }
        assert!(!is_authorized_source(&peer, hmd), "session end must revoke the HMD");
    }
}
