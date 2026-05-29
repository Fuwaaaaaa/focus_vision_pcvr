//! JSON-driven scenario runner for headless full-feature regression.
//!
//! Parses `tests/scenarios/*.json` via serde_json, drives the engine and
//! a mock client (plus optional tracking sender), and aggregates assertion
//! results into a `ScenarioReport`. Each scenario must run under
//! `--test-threads=1` because the engine claims a process-global
//! `status.json` for PIN discovery.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fvp_common::protocol::VideoCodec;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::engine::{EncodedFrame, StreamingEngine};
use crate::metrics::latency::FrameTimestamps;
use crate::simulator::face_sender::FaceMode;
use crate::simulator::test_helpers::{
    delete_stale_status, pick_free_ports, pick_free_udp_port_excluding, wait_for_pin,
};
use crate::simulator::tracking_sender::{PoseMode, TrackingSender};
use crate::simulator::{run as run_mock_client, MockClientConfig, MockClientError, MockClientStats};
use crate::video::synthetic_nal::SyntheticNalStream;

/// One scenario definition, parsed from `tests/scenarios/<name>.json`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub timeout_sec: u64,
    #[serde(default)]
    pub config_overrides: serde_json::Map<String, serde_json::Value>,
    pub client: ClientConfig,
    /// Future PRs will type these (TCP disconnect, packet loss). For now
    /// they parse as raw JSON so existing scenarios can reference fault
    /// entries without breaking the parser.
    #[serde(default)]
    pub fault_injection: Vec<serde_json::Value>,
    /// Engine-side stimuli scheduled at `at_sec` offsets from scenario
    /// start. PR-4 adds `queue_haptic`; future PRs may add config update,
    /// IDR request, etc.
    #[serde(default)]
    pub stimuli: Vec<Stimulus>,
    pub assertions: Assertions,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stimulus {
    /// Inject a haptic event at the engine. Routes through the same
    /// `engine::queue_haptic` path the SteamVR driver uses, so the
    /// engine then emits a `HAPTIC_EVENT (0x38)` TCP message to the
    /// mock client — exercising the full PC → HMD haptic path.
    QueueHaptic {
        at_sec: f64,
        controller_id: u8,
        duration_ms: u16,
        freq: f32,
        amp: f32,
    },
    /// Set the simulator UDP loss probability (0..=100). Use with
    /// `ClearLossPct` to inject loss for a bounded window.
    SetLossPct { at_sec: f64, pct: u8 },
    /// Reset the simulator UDP loss probability to 0.
    ClearLossPct { at_sec: f64 },
    /// Add `latency_us` of extra delay to every subsequent synthetic frame
    /// submission. Models OpenXR `xrWaitFrame` jitter that the production
    /// driver pays before pushing a frame to the encoder. Use with
    /// `ClearFrameLatency` to bound the injected window. Adaptive bitrate
    /// and GCC paths should react by lowering bitrate when latency climbs.
    InjectFrameLatency { at_sec: f64, latency_us: u64 },
    /// Reset the synthetic frame producer delay to 0.
    ClearFrameLatency { at_sec: f64 },
}

/// Extra per-frame delay (microseconds) the synthetic frame producer applies
/// after each `engine.submit_frame()` call. Mutated by
/// `Stimulus::InjectFrameLatency` and `Stimulus::ClearFrameLatency`; read
/// inside the frame producer loop in `run_scenario`.
static FRAME_LATENCY_US: AtomicU64 = AtomicU64::new(0);

impl Stimulus {
    fn at_sec(&self) -> f64 {
        match self {
            Self::QueueHaptic { at_sec, .. } => *at_sec,
            Self::SetLossPct { at_sec, .. } => *at_sec,
            Self::ClearLossPct { at_sec } => *at_sec,
            Self::InjectFrameLatency { at_sec, .. } => *at_sec,
            Self::ClearFrameLatency { at_sec } => *at_sec,
        }
    }

    fn execute(&self) {
        match *self {
            Self::QueueHaptic {
                controller_id,
                duration_ms,
                freq,
                amp,
                ..
            } => {
                crate::engine::queue_haptic(controller_id, duration_ms, freq, amp);
            }
            Self::SetLossPct { pct, .. } => {
                crate::transport::udp::set_simulator_loss_pct(pct);
                log::info!("scenario stimulus: loss_pct -> {}", pct);
            }
            Self::ClearLossPct { .. } => {
                crate::transport::udp::set_simulator_loss_pct(0);
                log::info!("scenario stimulus: loss_pct -> 0");
            }
            Self::InjectFrameLatency { latency_us, .. } => {
                FRAME_LATENCY_US.store(latency_us, Ordering::Relaxed);
                log::info!("scenario stimulus: frame_latency -> {} us", latency_us);
            }
            Self::ClearFrameLatency { .. } => {
                FRAME_LATENCY_US.store(0, Ordering::Relaxed);
                log::info!("scenario stimulus: frame_latency -> 0");
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub pin_source: PinSource,
    pub duration_sec: f64,
    #[serde(default)]
    pub tracking_pattern: Option<TrackingPatternSpec>,
    /// When set, the mock client periodically emits FACE_DATA (0x35) on
    /// the TCP control channel and the runner allocates a free UDP port
    /// to capture the engine's OSC output.
    #[serde(default)]
    pub face_pattern: Option<FacePatternSpec>,
    #[serde(default)]
    pub capture_haptic: bool,
    /// When true, the mock client counts SLEEP_ENTER/SLEEP_EXIT messages
    /// sent by the engine into `sleep_enter_count` / `sleep_exit_count`.
    #[serde(default)]
    pub capture_sleep_events: bool,
    /// When true, mock client records per-frame depacketization wall-time
    /// and reports p50/p95/p99 in stats. Use with
    /// `Assertions.max_decode_latency_us_p99` to gate codec comparison
    /// scenarios.
    #[serde(default)]
    pub measure_decode_latency: bool,
    /// When true, the mock client binds the audio UDP port and counts incoming
    /// Opus RTP packets into `audio_packets_received`. Pair with
    /// `audio.enabled = true` + `audio.synthetic_source` in `config_overrides`
    /// and `Assertions.min_audio_packets` to gate the audio path.
    #[serde(default)]
    pub receive_audio: bool,
    /// Manual override for the OSC loopback port. Most scenarios should
    /// leave this `null` and let the runner allocate dynamically — the
    /// runner mirrors the chosen port to both `face_tracking.osc_port`
    /// and the loopback bind.
    #[serde(default)]
    pub osc_listen_port: Option<u16>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PinSource {
    /// Read the engine-published PIN from status.json (golden path).
    StatusJson,
    /// Use a guaranteed-wrong PIN to exercise the PIN rejection branch.
    Wrong,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum TrackingPatternSpec {
    Still,
    SineWave { amp_m: f32, hz: f32 },
}

impl TrackingPatternSpec {
    fn into_pose_mode(self) -> PoseMode {
        match self {
            Self::Still => PoseMode::still_origin(),
            Self::SineWave { amp_m, hz } => PoseMode::SineWave {
                base: PoseMode::default_head(),
                amp_m,
                hz,
                left: None,
                right: None,
            },
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub enum FacePatternSpec {
    Relax,
    Exaggerate,
    SineSweep { hz: f32 },
    Blink { hz: f32 },
    Talk { hz: f32 },
    Smile { intensity: f32 },
    Frown { intensity: f32 },
}

impl FacePatternSpec {
    fn into_face_mode(self) -> FaceMode {
        match self {
            Self::Relax => FaceMode::Relax,
            Self::Exaggerate => FaceMode::Exaggerate,
            Self::SineSweep { hz } => FaceMode::SineSweep { hz },
            Self::Blink { hz } => FaceMode::Blink { hz },
            Self::Talk { hz } => FaceMode::Talk { hz },
            Self::Smile { intensity } => FaceMode::Smile { intensity },
            Self::Frown { intensity } => FaceMode::Frown { intensity },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Assertions {
    pub min_frames_decoded: Option<u64>,
    pub min_idr_frames: Option<u64>,
    pub min_heartbeats_sent: Option<u64>,
    pub min_video_packets: Option<u64>,
    /// Lower bound on `audio_packets_received` (Opus RTP, PT=111). Requires
    /// `client.receive_audio = true` and a synthetic audio source.
    pub min_audio_packets: Option<u64>,
    pub max_connect_duration_ms: Option<u64>,
    pub expect_pin_rejected: Option<bool>,
    /// Required OSC addresses (engine → loopback). Each entry must appear
    /// as a key in `MockClientStats::osc_messages`.
    pub expect_osc_addresses: Option<Vec<String>>,
    /// Lower bound on total OSC messages summed across all addresses.
    pub min_osc_messages: Option<u64>,
    /// Lower bound on `face_messages_sent` (mock-client → engine).
    pub min_face_messages_sent: Option<u64>,
    /// Lower bound on `haptic_events_received` (mock-client decoded).
    pub min_haptic_events_received: Option<u64>,
    /// Lower bound on `sleep_enter_count` (engine-emitted SLEEP_ENTER).
    pub min_sleep_enter_count: Option<u64>,
    /// Lower bound on `sleep_exit_count` (engine-emitted SLEEP_EXIT).
    pub min_sleep_exit_count: Option<u64>,
    /// Upper bound on `video_packets_received`. Useful for verifying
    /// that injected packet loss actually reduces throughput.
    pub max_video_packets: Option<u64>,
    /// Verify recording output. After the scenario, scan `dir` for files
    /// ending in `.h265` / `.h264` / `.wav` and assert their cumulative
    /// size meets `min_bytes`.
    pub expect_recording_files: Option<RecordingCheck>,
    /// Upper bound on the 99th-percentile depacketization wall-time
    /// reported by the mock client (microseconds). Only meaningful when
    /// `client.measure_decode_latency = true`.
    pub max_decode_latency_us_p99: Option<u32>,
    /// Lower bound on the number of decode-latency samples collected. Use
    /// to guard against codec scenarios where too few frames complete to
    /// make the percentile meaningful.
    pub min_decode_latency_samples: Option<u64>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RecordingCheck {
    /// Directory the engine wrote recordings into. Typically matches
    /// `config_overrides["recording.output_dir"]`.
    pub dir: String,
    /// Minimum cumulative size of recorded `.h264/.h265/.wav` files.
    pub min_bytes: u64,
}

/// Outcome of a single scenario run. `passed` is false iff `failures` is non-empty.
#[derive(Debug)]
pub struct ScenarioReport {
    pub name: String,
    pub passed: bool,
    pub failures: Vec<String>,
    pub stats: Option<MockClientStats>,
    pub duration: Duration,
}

impl ScenarioReport {
    pub fn assert_passed(&self) {
        if !self.passed {
            panic!(
                "scenario '{}' failed in {:?}:\n  - {}",
                self.name,
                self.duration,
                self.failures.join("\n  - "),
            );
        }
    }
}

#[derive(Debug)]
pub enum ScenarioError {
    Io(std::io::Error),
    Parse(String),
    Engine(String),
}

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O: {}", e),
            Self::Parse(s) => write!(f, "parse: {}", s),
            Self::Engine(s) => write!(f, "engine: {}", s),
        }
    }
}
impl std::error::Error for ScenarioError {}
impl From<std::io::Error> for ScenarioError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

pub fn parse_scenario_str(s: &str) -> Result<Scenario, ScenarioError> {
    serde_json::from_str(s).map_err(|e| ScenarioError::Parse(e.to_string()))
}

pub fn parse_scenario_file(path: &Path) -> Result<Scenario, ScenarioError> {
    let content = std::fs::read_to_string(path)?;
    parse_scenario_str(&content)
}

/// Run a scenario synchronously. Spins up a tokio runtime for the mock
/// client and tracking sender; the engine owns its own threads internally.
/// Returns a report regardless of pass/fail — caller decides what to do.
pub fn run_scenario(scenario: &Scenario) -> Result<ScenarioReport, ScenarioError> {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn"),
    )
    .is_test(true)
    .try_init();

    delete_stale_status();
    // Reset any simulator UDP loss left over from a prior scenario so the
    // run begins on a clean network. Stimuli inside the scenario can
    // re-arm loss as they fire.
    crate::transport::udp::set_simulator_loss_pct(0);
    // If the scenario asserts on recording output, wipe the target dir
    // before the engine opens its file so the assertion only counts THIS
    // run's bytes.
    if let Some(check) = &scenario.assertions.expect_recording_files {
        let _ = std::fs::remove_dir_all(&check.dir);
    }
    let (tcp_port, udp_port) = pick_free_ports();

    let mut config = AppConfig::default();
    config.network.tcp_port = tcp_port;
    config.network.udp_port = udp_port;
    apply_config_overrides(&mut config, &scenario.config_overrides)?;

    // When the scenario emits face data, allocate a free UDP port that
    // doesn't collide with the mock client's other UDP binds (video at
    // udp+1, tracking at udp+2, audio at udp+3) and route both the
    // engine's OSC bridge and the loopback receiver to it. Without the
    // exclude list, Windows allocates ephemeral ports sequentially and
    // the auto-pick often lands on udp+1 — which would silently swallow
    // RTP packets meant for the video receiver.
    let reserved_ports = [
        udp_port,
        udp_port + fvp_common::VIDEO_PORT_OFFSET,
        udp_port + fvp_common::AUDIO_PORT_OFFSET,
        udp_port + fvp_common::TRACKING_PORT_OFFSET,
    ];
    let osc_port: Option<u16> = if scenario.client.face_pattern.is_some() {
        Some(
            scenario
                .client
                .osc_listen_port
                .unwrap_or_else(|| pick_free_udp_port_excluding(&reserved_ports)),
        )
    } else {
        scenario.client.osc_listen_port
    };
    if let Some(p) = osc_port {
        config.face_tracking.osc_port = p;
        log::debug!(
            "scenario '{}': OSC port {} (reserved: {:?})",
            scenario.name, p, reserved_ports
        );
    }

    let framerate = config.video.framerate.max(1);
    let engine = StreamingEngine::new(config)
        .map_err(|e| ScenarioError::Engine(format!("{:?}", e)))?;

    let pin = match scenario.client.pin_source {
        PinSource::StatusJson => wait_for_pin(Duration::from_secs(5))
            .ok_or_else(|| ScenarioError::Engine("engine never published a PIN".into()))?,
        PinSource::Wrong => {
            let real = wait_for_pin(Duration::from_secs(5))
                .ok_or_else(|| ScenarioError::Engine("engine never published a PIN".into()))?;
            (real.wrapping_add(123_456)) % 1_000_000
        }
    };

    let mut client_config = MockClientConfig::from_ports(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        tcp_port,
        udp_port,
        pin,
    );
    let client_duration = Duration::from_secs_f64(scenario.client.duration_sec);
    client_config.duration = Some(client_duration);
    client_config.face_pattern = scenario.client.face_pattern.map(|p| p.into_face_mode());
    client_config.osc_listen_port = osc_port;
    client_config.capture_haptic = scenario.client.capture_haptic;
    client_config.capture_sleep_events = scenario.client.capture_sleep_events;
    client_config.measure_decode_latency = scenario.client.measure_decode_latency;
    client_config.receive_audio = scenario.client.receive_audio;

    let tracking_target = client_config.tracking_target;
    let tracking_spec = scenario.client.tracking_pattern.clone();

    let cancel = CancellationToken::new();
    let cancel_for_client = cancel.clone();

    // Stimuli scheduler: a dedicated OS thread fires each entry at its
    // `at_sec` offset by calling the same `engine::queue_haptic` etc. the
    // production SteamVR driver path uses. Sort by `at_sec` so out-of-
    // order JSON entries still fire in time.
    let mut stimuli: Vec<Stimulus> = scenario.stimuli.clone();
    stimuli.sort_by(|a, b| {
        a.at_sec()
            .partial_cmp(&b.at_sec())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let stimuli_thread = if stimuli.is_empty() {
        None
    } else {
        let stimuli_cancel = cancel.clone();
        let stimuli_start = Instant::now();
        Some(std::thread::spawn(move || {
            for stim in stimuli {
                if stimuli_cancel.is_cancelled() {
                    break;
                }
                let target = Duration::from_secs_f64(stim.at_sec());
                let elapsed = stimuli_start.elapsed();
                if target > elapsed {
                    std::thread::sleep(target - elapsed);
                }
                if stimuli_cancel.is_cancelled() {
                    break;
                }
                stim.execute();
            }
        }))
    };

    let client_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let tracking_handle = if let Some(spec) = tracking_spec {
                let cancel_for_tracking = cancel_for_client.clone();
                let mode = spec.into_pose_mode();
                match TrackingSender::new(tracking_target).await {
                    Ok(sender) => Some(tokio::spawn(async move {
                        sender.run(mode, 90, cancel_for_tracking).await;
                    })),
                    Err(e) => {
                        log::warn!("tracking sender bind failed: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            let result = run_mock_client(client_config, cancel_for_client).await;

            if let Some(h) = tracking_handle {
                h.abort();
                let _ = h.await;
            }
            result
        })
    });

    // Pump synthetic frames into the engine for the scenario duration.
    // Reset any frame-latency injection left over from a prior scenario so
    // this run starts unperturbed; InjectFrameLatency stimuli inside the
    // scenario re-arm it.
    FRAME_LATENCY_US.store(0, Ordering::Relaxed);
    let codec = match config_video_codec(&scenario.config_overrides) {
        Some(c) => c,
        None => VideoCodec::H265,
    };
    let stream_window = client_duration + Duration::from_millis(200);
    let mut stream = SyntheticNalStream::new(codec, framerate);
    let frame_period = Duration::from_secs_f64(1.0 / framerate as f64);
    let start = Instant::now();
    let mut next_tick = start;
    while start.elapsed() < stream_window {
        let synth = stream.next_frame();
        let frame = EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: synth.is_idr,
            timestamps: FrameTimestamps::new(synth.frame_index),
        };
        let _ = engine.submit_frame(frame);
        // InjectFrameLatency stimulus stretches each frame's effective
        // period — the next submission slides right by `extra_us`. The
        // engine's adaptive bitrate / GCC controllers should observe the
        // delay through HEARTBEAT_ACK and react.
        let extra_us = FRAME_LATENCY_US.load(Ordering::Relaxed);
        next_tick += frame_period + Duration::from_micros(extra_us);
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }

    cancel.cancel();
    if let Some(handle) = stimuli_thread {
        let _ = handle.join();
    }
    let stats_result = client_thread
        .join()
        .map_err(|_| ScenarioError::Engine("client thread panicked".into()))?;
    engine.shutdown();
    // Drop the engine BEFORE sleeping so the tokio runtime begins teardown
    // and the audio capture thread observes its cancel token. Without the
    // explicit drop we'd hold the engine in scope through the sleep, which
    // delays runtime drop until function return — leading to the next
    // scenario in the test process racing to acquire WASAPI resources the
    // previous engine still owns.
    drop(engine);
    // Give Windows time to release the WASAPI loopback device and any
    // lingering UDP socket state before the next scenario starts. 500 ms
    // is empirically enough on the test runner; the alternative is process
    // isolation (cargo nextest) which we don't require in CI today.
    std::thread::sleep(Duration::from_millis(500));

    let total_duration = start.elapsed();

    let (stats_opt, pin_rejected) = match stats_result {
        Ok(stats) => (Some(stats), false),
        Err(MockClientError::PinRejected) => (None, true),
        Err(other) => {
            return Ok(ScenarioReport {
                name: scenario.name.clone(),
                passed: false,
                failures: vec![format!("mock client error: {}", other)],
                stats: None,
                duration: total_duration,
            });
        }
    };

    let failures = evaluate_assertions(&scenario.assertions, stats_opt.as_ref(), pin_rejected);
    Ok(ScenarioReport {
        name: scenario.name.clone(),
        passed: failures.is_empty(),
        failures,
        stats: stats_opt,
        duration: total_duration,
    })
}

/// Apply dot-path overrides to an `AppConfig`, e.g. `"video.framerate": 60`.
/// Round-trips through `serde_json::Value` so any field declared by the
/// AppConfig serde derive is reachable.
fn apply_config_overrides(
    config: &mut AppConfig,
    overrides: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ScenarioError> {
    if overrides.is_empty() {
        return Ok(());
    }
    let mut json = serde_json::to_value(&*config)
        .map_err(|e| ScenarioError::Parse(format!("config to JSON: {}", e)))?;
    for (path, value) in overrides {
        set_at_path(&mut json, path, value.clone())
            .map_err(|e| ScenarioError::Parse(format!("override '{}': {}", path, e)))?;
    }
    *config = serde_json::from_value(json)
        .map_err(|e| ScenarioError::Parse(format!("config from JSON: {}", e)))?;
    Ok(())
}

fn set_at_path(
    json: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = json;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            let obj = cur
                .as_object_mut()
                .ok_or_else(|| format!("path segment '{}' is not an object", part))?;
            obj.insert((*part).to_string(), value.clone());
            return Ok(());
        } else {
            cur = cur
                .get_mut(*part)
                .ok_or_else(|| format!("path segment '{}' not found", part))?;
        }
    }
    Ok(())
}

fn evaluate_assertions(
    a: &Assertions,
    stats: Option<&MockClientStats>,
    pin_rejected: bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    if let Some(expect) = a.expect_pin_rejected {
        if expect != pin_rejected {
            failures.push(format!(
                "expect_pin_rejected={}, observed pin_rejected={}",
                expect, pin_rejected
            ));
        }
    }
    let Some(s) = stats else {
        // No stats available (handshake failed). Skip count-based checks.
        return failures;
    };
    if let Some(min) = a.min_frames_decoded {
        if s.frames_decoded < min {
            failures.push(format!(
                "min_frames_decoded: expected >= {}, got {}",
                min, s.frames_decoded
            ));
        }
    }
    if let Some(min) = a.min_idr_frames {
        if s.idr_frames_seen < min {
            failures.push(format!(
                "min_idr_frames: expected >= {}, got {}",
                min, s.idr_frames_seen
            ));
        }
    }
    if let Some(min) = a.min_heartbeats_sent {
        if s.heartbeats_sent < min {
            failures.push(format!(
                "min_heartbeats_sent: expected >= {}, got {}",
                min, s.heartbeats_sent
            ));
        }
    }
    if let Some(min) = a.min_video_packets {
        if s.video_packets_received < min {
            failures.push(format!(
                "min_video_packets: expected >= {}, got {}",
                min, s.video_packets_received
            ));
        }
    }
    if let Some(min) = a.min_audio_packets {
        if s.audio_packets_received < min {
            failures.push(format!(
                "min_audio_packets: expected >= {}, got {} (was receive_audio + synthetic_source on?)",
                min, s.audio_packets_received
            ));
        }
    }
    if let Some(max_ms) = a.max_connect_duration_ms {
        if s.connect_duration > Duration::from_millis(max_ms) {
            failures.push(format!(
                "max_connect_duration: expected <= {} ms, got {:?}",
                max_ms, s.connect_duration
            ));
        }
    }
    if let Some(min) = a.min_face_messages_sent {
        if s.face_messages_sent < min {
            failures.push(format!(
                "min_face_messages_sent: expected >= {}, got {}",
                min, s.face_messages_sent
            ));
        }
    }
    if let Some(min) = a.min_haptic_events_received {
        let received = s.haptic_events_received.len() as u64;
        if received < min {
            failures.push(format!(
                "min_haptic_events_received: expected >= {}, got {}",
                min, received
            ));
        }
    }
    if let Some(min) = a.min_sleep_enter_count {
        if s.sleep_enter_count < min {
            failures.push(format!(
                "min_sleep_enter_count: expected >= {}, got {}",
                min, s.sleep_enter_count
            ));
        }
    }
    if let Some(min) = a.min_sleep_exit_count {
        if s.sleep_exit_count < min {
            failures.push(format!(
                "min_sleep_exit_count: expected >= {}, got {}",
                min, s.sleep_exit_count
            ));
        }
    }
    if let Some(expected) = &a.expect_osc_addresses {
        for addr in expected {
            if !s.osc_messages.contains_key(addr) {
                failures.push(format!("expect_osc_addresses: missing '{}'", addr));
            }
        }
    }
    if let Some(min) = a.min_osc_messages {
        let total: u64 = s.osc_messages.values().map(|v| v.len() as u64).sum();
        if total < min {
            failures.push(format!(
                "min_osc_messages: expected >= {}, got {}",
                min, total
            ));
        }
    }
    if let Some(max) = a.max_video_packets {
        if s.video_packets_received > max {
            failures.push(format!(
                "max_video_packets: expected <= {}, got {}",
                max, s.video_packets_received
            ));
        }
    }
    if let Some(check) = &a.expect_recording_files {
        match measure_recording_dir(std::path::Path::new(&check.dir)) {
            Ok(total) if total >= check.min_bytes => {}
            Ok(total) => failures.push(format!(
                "expect_recording_files: {} total bytes in {}, want >= {}",
                total, check.dir, check.min_bytes
            )),
            Err(e) => failures.push(format!(
                "expect_recording_files: failed to read '{}': {}",
                check.dir, e
            )),
        }
    }
    if let Some(min) = a.min_decode_latency_samples {
        if s.depacketize_samples_count < min {
            failures.push(format!(
                "min_decode_latency_samples: expected >= {}, got {} (was measure_decode_latency on?)",
                min, s.depacketize_samples_count
            ));
        }
    }
    if let Some(max) = a.max_decode_latency_us_p99 {
        if s.depacketize_latency_us_p99 > max {
            failures.push(format!(
                "max_decode_latency_us_p99: expected <= {}, got {}",
                max, s.depacketize_latency_us_p99
            ));
        }
    }
    failures
}

/// Resolve the `video.codec` override (if any) into a `VideoCodec`. Falls
/// back to `None` so the caller can pick H.265 as the default.
fn config_video_codec(
    overrides: &serde_json::Map<String, serde_json::Value>,
) -> Option<VideoCodec> {
    let v = overrides.get("video.codec")?.as_str()?;
    match v.to_ascii_lowercase().as_str() {
        "h264" | "avc" => Some(VideoCodec::H264),
        "h265" | "hevc" => Some(VideoCodec::H265),
        _ => None,
    }
}

/// Sum the byte sizes of `.h265 / .h264 / .wav` files directly inside
/// `dir` (non-recursive). Returns 0 if the directory doesn't exist.
fn measure_recording_dir(dir: &std::path::Path) -> std::io::Result<u64> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut total: u64 = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if matches!(ext, "h265" | "h264" | "wav") {
            if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_scenario() {
        let json = r#"{
            "name": "test",
            "timeout_sec": 10,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0
            },
            "assertions": {}
        }"#;
        let s = parse_scenario_str(json).expect("parse");
        assert_eq!(s.name, "test");
        assert!(matches!(s.client.pin_source, PinSource::StatusJson));
        assert!(s.client.tracking_pattern.is_none());
    }

    #[test]
    fn parse_with_tracking_pattern_still() {
        let json = r#"{
            "name": "t",
            "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0,
                "tracking_pattern": { "kind": "still" }
            },
            "assertions": {}
        }"#;
        let s = parse_scenario_str(json).expect("parse");
        assert!(matches!(s.client.tracking_pattern, Some(TrackingPatternSpec::Still)));
    }

    #[test]
    fn parse_with_tracking_pattern_sine_wave() {
        let json = r#"{
            "name": "t",
            "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0,
                "tracking_pattern": { "kind": "sine_wave", "amp_m": 0.05, "hz": 0.5 }
            },
            "assertions": {}
        }"#;
        let s = parse_scenario_str(json).expect("parse");
        assert!(matches!(
            s.client.tracking_pattern,
            Some(TrackingPatternSpec::SineWave { .. })
        ));
    }

    #[test]
    fn parse_rejects_unknown_top_level_field() {
        let json = r#"{
            "name": "x", "timeout_sec": 5,
            "client": { "pin_source": "status_json", "duration_sec": 1.0 },
            "assertions": {},
            "bogus": 1
        }"#;
        assert!(parse_scenario_str(json).is_err());
    }

    #[test]
    fn parse_rejects_unknown_tracking_pattern_kind() {
        let json = r#"{
            "name": "x", "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0,
                "tracking_pattern": { "kind": "nope" }
            },
            "assertions": {}
        }"#;
        assert!(parse_scenario_str(json).is_err());
    }

    #[test]
    fn parse_with_face_pattern_exaggerate() {
        let json = r#"{
            "name": "ft", "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0,
                "face_pattern": { "kind": "exaggerate" }
            },
            "assertions": {}
        }"#;
        let s = parse_scenario_str(json).expect("parse");
        assert!(matches!(s.client.face_pattern, Some(FacePatternSpec::Exaggerate)));
    }

    #[test]
    fn parse_with_face_pattern_sine_sweep() {
        let json = r#"{
            "name": "ft", "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0,
                "face_pattern": { "kind": "sine_sweep", "hz": 2.0 }
            },
            "assertions": { "min_osc_messages": 50 }
        }"#;
        let s = parse_scenario_str(json).expect("parse");
        assert!(matches!(s.client.face_pattern, Some(FacePatternSpec::SineSweep { .. })));
        assert_eq!(s.assertions.min_osc_messages, Some(50));
    }

    #[test]
    fn parse_stimulus_queue_haptic() {
        let json = r#"{
            "name": "h", "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 2.0,
                "capture_haptic": true
            },
            "stimuli": [
                {
                    "kind": "queue_haptic",
                    "at_sec": 0.5,
                    "controller_id": 0,
                    "duration_ms": 100,
                    "freq": 160.0,
                    "amp": 0.7
                }
            ],
            "assertions": { "min_haptic_events_received": 1 }
        }"#;
        let s = parse_scenario_str(json).expect("parse");
        assert_eq!(s.stimuli.len(), 1);
        match s.stimuli[0] {
            Stimulus::QueueHaptic { controller_id, duration_ms, .. } => {
                assert_eq!(controller_id, 0);
                assert_eq!(duration_ms, 100);
            }
            other => panic!("expected QueueHaptic, got {:?}", other),
        }
        assert_eq!(s.assertions.min_haptic_events_received, Some(1));
    }

    #[test]
    fn parse_rejects_unknown_stimulus_kind() {
        let json = r#"{
            "name": "x", "timeout_sec": 5,
            "client": { "pin_source": "status_json", "duration_sec": 1.0 },
            "stimuli": [{ "kind": "blast_off", "at_sec": 0.0 }],
            "assertions": {}
        }"#;
        assert!(parse_scenario_str(json).is_err());
    }

    #[test]
    fn assertions_min_haptic_below_fails() {
        let a = Assertions {
            min_haptic_events_received: Some(2),
            ..Default::default()
        };
        let stats = MockClientStats {
            haptic_events_received: vec![crate::engine::HapticEvent {
                controller_id: 0,
                duration_ms: 100,
                frequency: 160.0,
                amplitude: 0.7,
            }],
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("min_haptic_events_received"));
    }

    #[test]
    fn parse_rejects_unknown_face_pattern_kind() {
        let json = r#"{
            "name": "x", "timeout_sec": 5,
            "client": {
                "pin_source": "status_json",
                "duration_sec": 1.0,
                "face_pattern": { "kind": "nope" }
            },
            "assertions": {}
        }"#;
        assert!(parse_scenario_str(json).is_err());
    }

    #[test]
    fn assertions_expect_osc_addresses_missing_fails() {
        let a = Assertions {
            expect_osc_addresses: Some(vec!["/avatar/parameters/JawOpen".to_string()]),
            ..Default::default()
        };
        let stats = MockClientStats::default();
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("JawOpen"));
    }

    #[test]
    fn assertions_expect_osc_addresses_present_passes() {
        let a = Assertions {
            expect_osc_addresses: Some(vec!["/avatar/parameters/JawOpen".to_string()]),
            ..Default::default()
        };
        let mut osc = crate::simulator::osc_loopback::OscCapture::new();
        osc.insert("/avatar/parameters/JawOpen".to_string(), vec![0.5]);
        let stats = MockClientStats {
            osc_messages: osc,
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert!(failures.is_empty());
    }

    #[test]
    fn assertions_min_osc_messages_total_summed() {
        let a = Assertions {
            min_osc_messages: Some(5),
            ..Default::default()
        };
        let mut osc = crate::simulator::osc_loopback::OscCapture::new();
        osc.insert("/a".to_string(), vec![0.1, 0.2, 0.3]);
        osc.insert("/b".to_string(), vec![0.4, 0.5]);
        let stats = MockClientStats {
            osc_messages: osc,
            ..Default::default()
        };
        // Total = 3 + 2 = 5, meets minimum
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert!(failures.is_empty());
    }

    #[test]
    fn apply_overrides_changes_video_framerate() {
        let mut config = AppConfig::default();
        let original = config.video.framerate;
        let mut overrides = serde_json::Map::new();
        overrides.insert("video.framerate".to_string(), serde_json::json!(72));
        apply_config_overrides(&mut config, &overrides).unwrap();
        assert_eq!(config.video.framerate, 72);
        assert_ne!(config.video.framerate, original);
    }

    #[test]
    fn apply_overrides_empty_is_noop() {
        let mut config = AppConfig::default();
        let original = format!("{:?}", config);
        apply_config_overrides(&mut config, &serde_json::Map::new()).unwrap();
        assert_eq!(format!("{:?}", config), original);
    }

    #[test]
    fn apply_overrides_rejects_bad_path() {
        let mut config = AppConfig::default();
        let mut overrides = serde_json::Map::new();
        overrides.insert("does.not.exist".to_string(), serde_json::json!(1));
        assert!(apply_config_overrides(&mut config, &overrides).is_err());
    }

    #[test]
    fn assertions_min_frames_below_fails() {
        let a = Assertions {
            min_frames_decoded: Some(100),
            ..Default::default()
        };
        let stats = MockClientStats {
            frames_decoded: 50,
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("min_frames_decoded"));
    }

    #[test]
    fn assertions_min_audio_packets_below_fails() {
        let a = Assertions {
            min_audio_packets: Some(50),
            ..Default::default()
        };
        let stats = MockClientStats {
            audio_packets_received: 10,
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("min_audio_packets"));
    }

    #[test]
    fn assertions_min_audio_packets_met_passes() {
        let a = Assertions {
            min_audio_packets: Some(50),
            ..Default::default()
        };
        let stats = MockClientStats {
            audio_packets_received: 200,
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, Some(&stats), false);
        assert!(failures.is_empty());
    }

    #[test]
    fn assertions_pin_rejected_match_passes() {
        let a = Assertions {
            expect_pin_rejected: Some(true),
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, None, true);
        assert!(failures.is_empty());
    }

    #[test]
    fn assertions_pin_rejected_mismatch_fails() {
        let a = Assertions {
            expect_pin_rejected: Some(true),
            ..Default::default()
        };
        let failures = evaluate_assertions(&a, None, false);
        assert_eq!(failures.len(), 1);
    }
}
