//! End-to-end loopback test: StreamingEngine + mock-client in one process.
//!
//! Validates that the full Rust-side pipeline (TCP+TLS handshake, PIN flow,
//! RTP packetization, FEC, UDP send, depacketization, HEARTBEAT_ACK round
//! trip) works without any external dependencies — no SteamVR, no NVENC,
//! no Focus Vision hardware, no Android client.
//!
//! Gated behind the `simulator` feature: `cargo test --features simulator
//! --test headless_e2e_test`. CI's headless-e2e job runs this exact target.

#![cfg(feature = "simulator")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use fvp_common::protocol::{ControllerState, VideoCodec};
use streaming_engine::config::AppConfig;
use streaming_engine::engine::{EncodedFrame, StreamingEngine};
use streaming_engine::metrics::latency::FrameTimestamps;
use streaming_engine::simulator::tracking_sender::{PoseMode, TrackingSender};
use streaming_engine::simulator::{run as run_mock_client, MockClientConfig, MockClientStats};
// Shared with sim.rs and the scenario runner. Reserves a contiguous,
// non-ephemeral port block so the engine's ephemeral sender sockets can't
// collide with the mock client's fixed video/audio receiver ports (see the
// helper's doc comment for the WSAEADDRINUSE failure mode it prevents).
use streaming_engine::simulator::test_helpers::pick_free_ports;
use streaming_engine::video::synthetic_nal::SyntheticNalStream;
use tokio_util::sync::CancellationToken;

/// Path to status.json. The engine writes here on each
/// TcpControlServer::new() call (see engine.rs::run_streaming).
fn status_path() -> Option<std::path::PathBuf> {
    dirs_next::data_dir().map(|d| d.join("FocusVisionPCVR").join("status.json"))
}

/// Delete any stale status.json before launching an engine. Without this
/// a prior test run leaves a file behind, and `wait_for_pin` happily reads
/// the OLD pin while the engine is still starting up.
fn delete_stale_status() {
    if let Some(p) = status_path() {
        let _ = std::fs::remove_file(&p);
    }
}

/// Poll status.json until it has a non-placeholder PIN, or time out.
/// Assumes the caller has already cleared any stale file via
/// `delete_stale_status()` so the value we read is definitely fresh.
fn wait_for_pin(timeout: Duration) -> Option<u32> {
    let path = status_path()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(s) = v["pin"].as_str() {
                    if s != "------" {
                        if let Ok(p) = s.parse::<u32>() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Build an `AppConfig` for the headless E2E that never touches real audio
/// hardware. With the default `synthetic_source = "off"` the engine falls back
/// to real WASAPI capture (`spawn_real_capture`), whose cpal `Stream` holds
/// COM/WASAPI device handles on a detached thread. Constructing a SECOND engine
/// in the same process (as `headless_e2e_resolution_scale_reduces_bandwidth`
/// does — two full lifecycles) then crashes that teardown on a headless CI
/// runner with no audio device (STATUS_ACCESS_VIOLATION, 0xc0000005). Selecting
/// synthetic "sine" audio keeps the full Opus-over-UDP path exercised while
/// matching `companion-app/src/sim.rs::load_sim_config`'s hardware-free
/// contract, so the simulator stays truly hardware-independent.
fn sim_test_config(tcp_port: u16, udp_port: u16) -> AppConfig {
    let mut config = AppConfig::default();
    config.network.tcp_port = tcp_port;
    config.network.udp_port = udp_port;
    config.audio.enabled = true;
    config.audio.synthetic_source = "sine".to_string();
    config
}

#[test]
fn headless_e2e_basic_video_flow() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true)
        .try_init();

    // 1. Pick ports, clear stale PIN, configure engine, launch.
    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    // The frames below come at 60/s whatever this says; status.json's fps
    // must be those, not this.
    config.video.framerate = 120;

    let engine = StreamingEngine::new(config.clone()).expect("engine new");

    // 2. Wait for the engine to publish the PIN. The async task that
    //    constructs TcpControlServer needs a moment to spin up.
    let pin = wait_for_pin(Duration::from_secs(3))
        .expect("engine never published a PIN to status.json");
    eprintln!("e2e: engine PIN = {:06}", pin);

    // 3. Construct mock-client config and run it in a thread that owns its
    //    own tokio runtime — we cannot drive run_mock_client.await from a
    //    plain #[test] without one.
    let server_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut client_config = MockClientConfig::from_ports(server_ip, tcp_port, udp_port, pin);
    client_config.duration = Some(Duration::from_secs(2));
    let cancel = CancellationToken::new();
    let cancel_for_client = cancel.clone();
    let client_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(run_mock_client(client_config, cancel_for_client))
    });

    // 4. Pump synthetic NAL frames into the engine for the duration of the
    //    mock-client run. The engine's internal channel has capacity 4; if
    //    we send faster than the network drains, submit_frame returns false.
    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60); // 1 IDR/sec
    let frame_period = Duration::from_secs_f64(1.0 / 60.0);
    let start = Instant::now();
    let mut frames_offered = 0u64;
    let mut frames_accepted = 0u64;
    let mut next_tick = start;
    // Regression guard for the engine->companion status contract: an active
    // session must publish status="streaming" with live fps. Before this was
    // wired the engine only ever wrote "waiting", so the companion's Connected
    // view (and all live stats) stayed dark -- in the sim AND on real hardware.
    // Poll *during* the session: the mock client disconnects at its 2 s
    // duration, after which the engine reverts status.json to "waiting".
    // REGRESSION: the fps was the configured framerate, not what was sent.
    let mut streaming_seen = false;
    let mut status_fps = 0;
    while start.elapsed() < Duration::from_millis(2200) {
        let synth = stream.next_frame();
        let frame = EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: synth.is_idr,
            timestamps: FrameTimestamps::new(synth.frame_index),
            render_orientation: None,
        };
        frames_offered += 1;
        if engine.submit_frame(frame) {
            frames_accepted += 1;
        }
        if frames_offered.is_multiple_of(6) {
            if let Some(v) = status_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            {
                let fps = v["fps"].as_u64().unwrap_or(0);
                if v["status"] == "streaming" && fps > 0 {
                    streaming_seen = true;
                    status_fps = status_fps.max(fps);
                }
            }
        }
        next_tick += frame_period;
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }

    assert!(streaming_seen,
        "engine must publish status=\"streaming\" with fps>0 during an active session");
    assert!((30..=75).contains(&status_fps),
        "status.json's fps must be the frames sent (60/s), not the configured 120: {status_fps}");

    // 5. Stop the mock-client (it would also stop on its --duration deadline,
    //    but cancelling makes the test deterministic).
    cancel.cancel();
    let stats = client_thread.join().expect("client thread join")
        .expect("mock-client run errored");
    engine.shutdown();

    eprintln!(
        "e2e: offered={} accepted={} packets={} frames={} IDR={} hb={}",
        frames_offered, frames_accepted,
        stats.video_packets_received, stats.frames_decoded,
        stats.idr_frames_seen, stats.heartbeats_sent,
    );

    // 6. Assertions. We don't pin exact counts because parallel test
    //    runs and shared frame channels make them noisy; instead we
    //    assert on coarse-grained invariants that prove the pipeline
    //    really is round-tripping bytes.
    assert!(stats.connect_duration < Duration::from_secs(1),
        "handshake should complete in well under a second, got {:?}",
        stats.connect_duration);
    assert!(stats.video_packets_received > 50,
        "expected >50 video packets across 2 s, got {}",
        stats.video_packets_received);
    assert!(stats.frames_decoded > 10,
        "expected >10 reassembled frames, got {}",
        stats.frames_decoded);
    assert!(stats.idr_frames_seen >= 1,
        "expected at least one keyframe reassembly, got {}",
        stats.idr_frames_seen);
    assert!(stats.heartbeats_sent >= 2,
        "expected >=2 heartbeats over 2 s @ 500 ms, got {}",
        stats.heartbeats_sent);
    assert!(stats.heartbeat_acks_received >= 2,
        "the engine must answer the heartbeats, got {} acks for {}",
        stats.heartbeat_acks_received, stats.heartbeats_sent);
    assert!(frames_accepted > 0,
        "engine should accept some submitted frames once the channel drains");
}

/// Run the full headless pipeline for `duration` at 60 fps with the engine
/// config adjusted by `configure`, feeding `stream`, and return the mock
/// client's stats.
fn run_pipeline(
    configure: impl FnOnce(&mut AppConfig),
    mut stream: SyntheticNalStream,
    duration: Duration,
) -> MockClientStats {
    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.framerate = 60;
    configure(&mut config);

    let engine = StreamingEngine::new(config.clone()).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");

    let server_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut client_config = MockClientConfig::from_ports(server_ip, tcp_port, udp_port, pin);
    client_config.duration = Some(duration);
    let cancel = CancellationToken::new();
    let cancel_for_client = cancel.clone();
    let client_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().unwrap();
        rt.block_on(run_mock_client(client_config, cancel_for_client))
    });

    pump_frames(&engine, &mut stream, duration + Duration::from_millis(200), || {});
    cancel.cancel();
    let stats = client_thread.join().expect("client thread join").expect("mock-client run");
    engine.shutdown();
    stats
}

/// Submit frames from `stream` to `engine` at 60 fps for `duration`,
/// calling `tick` after each one.
fn pump_frames(
    engine: &StreamingEngine,
    stream: &mut SyntheticNalStream,
    duration: Duration,
    mut tick: impl FnMut(),
) {
    let frame_period = Duration::from_secs_f64(1.0 / 60.0);
    let start = Instant::now();
    let mut next_tick = start;
    while start.elapsed() < duration {
        let synth = stream.next_frame();
        let frame = EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: synth.is_idr,
            timestamps: FrameTimestamps::new(synth.frame_index),
            render_orientation: None,
        };
        let _ = engine.submit_frame(frame);
        tick();
        next_tick += frame_period;
        let now = Instant::now();
        if next_tick > now { std::thread::sleep(next_tick - now); } else { next_tick = now; }
    }
}

/// Run a mock HMD on its own thread (and tokio runtime) until its
/// `duration` ends.
fn spawn_mock_client(
    config: MockClientConfig,
) -> std::thread::JoinHandle<Result<MockClientStats, streaming_engine::simulator::MockClientError>> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(run_mock_client(config, CancellationToken::new()))
    })
}

fn read_status() -> Option<serde_json::Value> {
    let content = std::fs::read_to_string(status_path()?).ok()?;
    serde_json::from_str(&content).ok()
}

/// Send the HMD's tracking (head pose, plus any controllers in `mode`) to
/// the engine at 90 Hz from its own thread until `cancel`. The source is
/// 127.0.0.1, the address the mock client pairs from.
fn spawn_tracking(target: SocketAddr, mode: PoseMode, cancel: CancellationToken) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let sender = TrackingSender::new(target).await.expect("tracking socket");
            sender.run(mode, 90, cancel).await;
        });
    })
}

/// What the driver's view-config callback last received.
static DRIVER_VIEW: std::sync::Mutex<Option<streaming_engine::FvpViewConfig>> = std::sync::Mutex::new(None);

extern "C" fn record_driver_view(view: *const streaming_engine::FvpViewConfig) {
    // SAFETY: the engine passes a pointer to a live FvpViewConfig for the
    // duration of the call.
    let view = unsafe { *view };
    *DRIVER_VIEW.lock().unwrap() = Some(view);
}

/// The headset's field of view and IPD (VIEW_CONFIG) reach the driver, which
/// sets SteamVR's projection from them. REGRESSION: SteamVR rendered with a
/// fixed 100° per eye whatever the headset displayed.
#[test]
fn headless_e2e_view_config_reaches_the_driver() {
    use fvp_common::protocol::{EyeFov, ViewConfig};
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    streaming_engine::engine::set_view_config_callback(record_driver_view);
    *DRIVER_VIEW.lock().unwrap() = None;
    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.framerate = 60;
    let engine = StreamingEngine::new(config).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");

    let deg = std::f32::consts::PI / 180.0;
    let view = ViewConfig {
        eyes: [
            EyeFov { left: -52.0 * deg, right: 45.0 * deg, up: 41.0 * deg, down: -49.0 * deg },
            EyeFov { left: -45.0 * deg, right: 52.0 * deg, up: 41.0 * deg, down: -49.0 * deg },
        ],
        ipd_m: 0.0635,
    };
    let mut hmd = MockClientConfig::from_ports(IpAddr::V4(Ipv4Addr::LOCALHOST), tcp_port, udp_port, pin);
    hmd.duration = Some(Duration::from_millis(1500));
    hmd.view_config = Some(view);
    let hmd = spawn_mock_client(hmd);
    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60);
    pump_frames(&engine, &mut stream, Duration::from_millis(1700), || {});
    hmd.join().unwrap().expect("session");
    engine.shutdown();

    let received = DRIVER_VIEW.lock().unwrap().expect("the driver never got the headset's view");
    assert_eq!(received, streaming_engine::FvpViewConfig::from(&view));
    assert_eq!(received.left_eye[0], -52.0 * deg, "left eye, angle left first");
    assert_eq!(received.right_eye[1], 52.0 * deg);
}

static IDR_REQUESTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static DRIVER_BITRATE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

extern "C" fn record_idr_request() {
    IDR_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

extern "C" fn record_bitrate(bps: u32) {
    DRIVER_BITRATE.store(bps, std::sync::atomic::Ordering::SeqCst);
}

/// A session starts clean. REGRESSION: the frames queued while no one was
/// connected went out first (stale, not a keyframe), the encoder kept the
/// previous session's bitrate (sleep's, after a nap), and no keyframe was
/// asked for.
#[test]
fn headless_e2e_session_starts_clean() {
    use std::sync::atomic::Ordering;
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    streaming_engine::engine::set_idr_callback(record_idr_request);
    streaming_engine::engine::set_bitrate_callback(record_bitrate);
    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.bitrate_mbps = 37;
    let engine = StreamingEngine::new(config).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");

    // Frames the driver submitted before the headset connected.
    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60);
    for _ in 0..4 {
        let synth = stream.next_frame();
        let _ = engine.submit_frame(EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: false,
            timestamps: FrameTimestamps::new(synth.frame_index),
            render_orientation: None,
        });
    }
    IDR_REQUESTS.store(0, Ordering::SeqCst);
    DRIVER_BITRATE.store(8_000_000, Ordering::SeqCst); // as a nap left it

    let mut hmd = MockClientConfig::from_ports(IpAddr::V4(Ipv4Addr::LOCALHOST), tcp_port, udp_port, pin);
    hmd.duration = Some(Duration::from_millis(800));
    let stats = spawn_mock_client(hmd).join().unwrap().expect("session");
    engine.shutdown();

    assert_eq!(stats.video_packets_received, 0, "the queued frames were stale and are dropped");
    assert_eq!(DRIVER_BITRATE.load(Ordering::SeqCst), 37_000_000, "the encoder is set to the bitrate setting");
    assert!(IDR_REQUESTS.load(Ordering::SeqCst) >= 1, "a keyframe is asked for");
}

/// Each frame reaches the headset with the head orientation it was rendered
/// at (v6), so the headset can turn it to where the head is when shown.
/// REGRESSION: the protocol had no way to tie a frame to its pose.
#[test]
fn headless_e2e_frames_carry_their_render_pose() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.framerate = 60;
    let engine = StreamingEngine::new(config).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");

    let mut hmd = MockClientConfig::from_ports(IpAddr::V4(Ipv4Addr::LOCALHOST), tcp_port, udp_port, pin);
    hmd.duration = Some(Duration::from_millis(1500));
    let hmd = spawn_mock_client(hmd);

    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60);
    let turn = |i: u32| {
        let half = i as f32 * 0.005;
        [0.0, half.sin(), 0.0, half.cos()]
    };
    let mut last_sent = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(1700) {
        let synth = stream.next_frame();
        let q = turn(synth.frame_index);
        if engine.submit_frame(EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: synth.is_idr,
            timestamps: FrameTimestamps::new(synth.frame_index),
            render_orientation: Some(q),
        }) {
            last_sent = Some(q);
        }
        std::thread::sleep(Duration::from_secs_f64(1.0 / 60.0));
    }
    let stats = hmd.join().unwrap().expect("session");
    engine.shutdown();

    assert!(stats.frames_decoded > 30, "frames flowed: {}", stats.frames_decoded);
    assert_eq!(stats.frames_with_render_pose, stats.frames_decoded, "every frame starts with its pose");
    let got = stats.last_render_orientation.expect("a known orientation");
    // The newest frame the client completed is one of the last ones sent.
    let sent = last_sent.unwrap();
    assert!((got[1] - sent[1]).abs() < 0.05 && got[0] == 0.0 && got[2] == 0.0, "{got:?} vs {sent:?}");
}

/// REGRESSION: the engine kept the HMD's last input forever. A controller
/// that lost tracking, or a session that ended, left its trigger held and
/// its stick pushed in SteamVR — and the headset's last pose valid.
#[test]
fn headless_e2e_stale_and_ended_hmd_input_is_released() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.framerate = 60;
    let engine = StreamingEngine::new(config).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");
    let server_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let tracking_target = SocketAddr::new(server_ip, udp_port + fvp_common::TRACKING_PORT_OFFSET);
    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60);

    let mut hmd = MockClientConfig::from_ports(server_ip, tcp_port, udp_port, pin);
    hmd.duration = Some(Duration::from_millis(3000));
    let hmd = spawn_mock_client(hmd);

    // 1. The right trigger is held.
    let held = ControllerState {
        controller_id: 1,
        orientation: [0.0, 0.0, 0.0, 1.0],
        trigger: 1.0,
        ..Default::default()
    };
    let with_controller = PoseMode::Still { head: PoseMode::default_head(), left: None, right: Some(held) };
    let controller_cancel = CancellationToken::new();
    let controller_tracking = spawn_tracking(tracking_target, with_controller, controller_cancel.clone());
    let mut held_seen = false;
    pump_frames(&engine, &mut stream, Duration::from_millis(1200), || {
        held_seen |= engine.get_controller(1).is_some_and(|c| c.trigger == 1.0);
    });
    assert!(held_seen, "the held trigger must reach the driver while the controller reports");

    // 2. The controller stops reporting (lost tracking); the head goes on.
    controller_cancel.cancel();
    controller_tracking.join().unwrap();
    let head_cancel = CancellationToken::new();
    let head_tracking = spawn_tracking(tracking_target, PoseMode::still_origin(), head_cancel.clone());
    pump_frames(&engine, &mut stream, Duration::from_millis(500), || {});
    assert!(engine.get_controller(1).is_none(),
        "a controller that stopped reporting must not keep its trigger held");
    assert!(engine.get_tracking().is_some(), "the head is still tracked");

    // 3. The session ends (DISCONNECT at the mock's 3 s). The head pose
    //    must go too, although tracking packets keep arriving.
    pump_frames(&engine, &mut stream, Duration::from_millis(1500), || {});
    hmd.join().unwrap().expect("session");
    let deadline = Instant::now() + Duration::from_secs(2);
    while engine.get_tracking().is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let pose_after_session = engine.get_tracking();
    head_cancel.cancel();
    head_tracking.join().unwrap();
    engine.shutdown();
    assert!(pose_after_session.is_none(), "the pose of an HMD that left must not stay valid");
}

/// REGRESSION: a link that died without a FIN/RST reaching the PC (Wi-Fi
/// gone, headset asleep) left the engine's control read pending forever —
/// the session kept streaming to nobody and turned away the HMD's
/// reconnects. The engine now drops a control connection that has been
/// silent for 3 s (six missed heartbeats).
#[test]
fn headless_e2e_silent_hmd_is_dropped() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.framerate = 60;
    let engine = StreamingEngine::new(config).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");
    let server_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let tracking_target = SocketAddr::new(server_ip, udp_port + fvp_common::TRACKING_PORT_OFFSET);

    // Heartbeats stop 1 s in; the connection stays open.
    let silent_after = Duration::from_secs(1);
    let mut hmd = MockClientConfig::from_ports(server_ip, tcp_port, udp_port, pin);
    hmd.duration = Some(Duration::from_secs(6));
    hmd.silent_after = Some(silent_after);
    let hmd = spawn_mock_client(hmd);
    // Tracking keeps arriving throughout: only the control channel died.
    let tracking_cancel = CancellationToken::new();
    let tracking = spawn_tracking(tracking_target, PoseMode::still_origin(), tracking_cancel.clone());

    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60);
    let mut tracked_while_alive = false;
    pump_frames(&engine, &mut stream, Duration::from_millis(6200), || {
        tracked_while_alive |= engine.get_tracking().is_some();
    });
    let stats = hmd.join().unwrap().expect("silent session");
    let pose_after_drop = engine.get_tracking();
    let status = read_status();
    tracking_cancel.cancel();
    tracking.join().unwrap();
    engine.shutdown();

    eprintln!("silent HMD: {} heartbeats, closed after {:?}", stats.heartbeats_sent, stats.control_closed_after);
    let closed = stats.control_closed_after.expect("the engine must drop a silent control connection");
    // The last heartbeat went out 0.5-1 s in; the limit is 3 s after it.
    assert!(closed >= Duration::from_secs(3),
        "dropped after {closed:?}: before the HMD was silent for the full 3 s");
    assert!(closed <= silent_after + Duration::from_secs(4),
        "dropped after {closed:?}: long after the 3 s limit");
    assert!(tracked_while_alive, "the session must take the HMD's tracking while it lasts");
    assert!(pose_after_drop.is_none(), "a dropped HMD's tracking must be refused and its pose cleared");
    let status = status.expect("status.json must exist");
    assert_eq!(status["status"], "waiting", "the engine must be waiting for the HMD again: {status}");
}

/// REGRESSION: when the link dropped (no DISCONNECT) and the HMD came back
/// inside the 5 s hold window, the engine accepted the connection, threw it
/// away and started over with a NEW PIN — which the HMD cannot know, so it
/// never got back in without the user pairing again. The hold now accepts
/// the session's PIN once more and streams on the reconnected connection.
#[test]
fn headless_e2e_reconnect_within_hold_keeps_pin_and_streams() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = sim_test_config(tcp_port, udp_port);
    config.video.framerate = 60;
    let engine = StreamingEngine::new(config).expect("engine new");
    let pin = wait_for_pin(Duration::from_secs(3)).expect("engine never published a PIN");
    let server_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut stream = SyntheticNalStream::new(VideoCodec::H265, 60);

    // 1. First connection streams, then the link drops without DISCONNECT.
    let mut first = MockClientConfig::from_ports(server_ip, tcp_port, udp_port, pin);
    first.duration = Some(Duration::from_millis(1500));
    first.abrupt_close = true;
    let first = spawn_mock_client(first);
    pump_frames(&engine, &mut stream, Duration::from_millis(1700), || {});
    let first = first.join().unwrap().expect("first session");
    assert!(first.frames_decoded > 5, "first session must stream, got {}", first.frames_decoded);

    // 2. The engine is in its hold window, still offering the same PIN.
    let deadline = Instant::now() + Duration::from_secs(3);
    let hold_status = loop {
        let v = read_status();
        if v.as_ref().is_some_and(|v| v["status"] == "waiting") || Instant::now() > deadline {
            break v;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let hold_status = hold_status.expect("status.json must exist");
    assert_eq!(hold_status["status"], "waiting", "engine must notice the drop: {hold_status}");
    assert_eq!(hold_status["pin"], format!("{pin:06}"), "hold must offer the session PIN");
    assert!(hold_status["pin_expires_in_seconds"].as_u64().unwrap_or(u64::MAX) <= 5,
        "the countdown shows the hold window: {hold_status}");

    // 3. The HMD comes back with the PIN it paired with and keeps streaming.
    let mut second = MockClientConfig::from_ports(server_ip, tcp_port, udp_port, pin);
    second.duration = Some(Duration::from_millis(1500));
    let second = spawn_mock_client(second);
    let mut streaming_seen = false;
    pump_frames(&engine, &mut stream, Duration::from_millis(1700), || {
        streaming_seen |= read_status().is_some_and(|v| v["status"] == "streaming");
    });
    let second = second.join().unwrap().expect("reconnect with the session PIN must succeed");
    engine.shutdown();

    eprintln!("reconnect: first={} frames, second={} frames",
        first.frames_decoded, second.frames_decoded);
    assert!(second.frames_decoded > 5, "reconnected session must stream, got {}", second.frames_decoded);
    assert!(streaming_seen, "status must return to \"streaming\" after the reconnect");
}

/// Run the pipeline at a given `resolution_scale`, feeding synthetic NALs
/// whose size scales with the encoded area, and return
/// `(video_bytes_received, frames_decoded)` measured by the mock client.
fn run_pipeline_video_bytes(resolution_scale: f32) -> (u64, u64) {
    let render = AppConfig::default().video.resolution_per_eye;
    let stream = SyntheticNalStream::new(VideoCodec::H265, 60)
        .with_resolution(render[0], render[1], resolution_scale);
    let stats = run_pipeline(
        |c| c.video.resolution_scale = resolution_scale, stream, Duration::from_secs(2),
    );
    (stats.video_bytes_received, stats.frames_decoded)
}

/// REGRESSION: an IDR whose slices were too big for one Reed-Solomon code
/// word went out with some slices empty, so it never reassembled. At 100 %
/// redundancy one code word holds 128 data shards, so this 320 KB IDR needs
/// 3 slices instead of the configured 2. (Real IDRs hit the same limit at
/// ~0.9-1.1 MB with 4 slices and the default 20-40 % redundancy; small code
/// words keep the engine's first encode — building the Reed-Solomon
/// matrices is slow without optimization — well inside the run.)
#[test]
fn headless_e2e_large_idr_is_delivered() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    let stream = SyntheticNalStream::new(VideoCodec::H265, 30).with_sizes(320_000, 4 * 1024);
    let stats = run_pipeline(
        |c| {
            c.network.fec_redundancy = 1.0;
            c.network.fec_redundancy_max = 1.0;
            c.network.adaptive_fec_enabled = false;
            c.network.slice_count = 2;
        },
        stream,
        Duration::from_secs(3),
    );
    eprintln!("large IDR: frames={} IDR={} packets={}",
        stats.frames_decoded, stats.idr_frames_seen, stats.video_packets_received);
    assert!(stats.idr_frames_seen >= 1,
        "a 320 KB IDR must reassemble, got {} IDRs of {} frames",
        stats.idr_frames_seen, stats.frames_decoded);
}

/// The verifiable core of Phase 0: a half-resolution encode genuinely puts fewer
/// bytes on the wire. Same pipeline, two scales. The *payload* is exactly a
/// quarter (proven deterministically by the synthetic_nal unit test); the *wire*
/// ratio is higher — typically ~0.4 — because fixed per-packet RTP/FVP headers
/// and FEC redundancy don't shrink with the payload, and the smaller IDR drops
/// below the 16 KB slice-FEC threshold into the bulk-FEC regime. The band is
/// chosen to prove a substantial (≳45%) reduction while tolerating that
/// overhead and run-to-run IDR-mix variance.
#[test]
fn headless_e2e_resolution_scale_reduces_bandwidth() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true).try_init();

    let (full_bytes, full_frames) = run_pipeline_video_bytes(1.0);
    let (half_bytes, half_frames) = run_pipeline_video_bytes(0.5);
    assert!(full_frames > 5 && half_frames > 5,
        "both runs must decode frames: full={full_frames} half={half_frames}");

    let full_bpf = full_bytes as f64 / full_frames as f64;
    let half_bpf = half_bytes as f64 / half_frames as f64;
    let ratio = half_bpf / full_bpf;
    eprintln!(
        "bandwidth: full={full_bytes}B/{full_frames}f={full_bpf:.0} B/frame, \
         half={half_bytes}B/{half_frames}f={half_bpf:.0} B/frame, ratio={ratio:.3}");

    assert!(half_bpf < full_bpf,
        "half-res must send fewer bytes per frame (full={full_bpf:.0}, half={half_bpf:.0})");
    assert!((0.20..0.50).contains(&ratio),
        "half-res per-frame wire bytes should be a substantial reduction over full-res \
         (~1/4 payload + fixed overhead → ~0.4); got ratio {ratio:.3}");
}

#[test]
fn headless_e2e_wrong_pin_rejected() {
    // Sanity check that the engine actually validates the PIN — if this
    // ever passes, somebody removed the security gate.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true)
        .try_init();

    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let config = sim_test_config(tcp_port, udp_port);
    let _engine = StreamingEngine::new(config).expect("engine new");

    let real_pin = wait_for_pin(Duration::from_secs(3))
        .expect("engine never published a PIN");
    // Pick a value that is guaranteed to differ from the random PIN.
    let wrong_pin = (real_pin.wrapping_add(123_456)) % 1_000_000;

    let mut cfg = MockClientConfig::from_ports(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        tcp_port,
        udp_port,
        wrong_pin,
    );
    cfg.duration = Some(Duration::from_millis(500));

    let cancel = CancellationToken::new();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(run_mock_client(cfg, cancel));
    match result {
        Err(streaming_engine::simulator::MockClientError::PinRejected) => {} // expected
        // Server may also bail with a generic protocol/I/O error if it tears
        // down the TLS session before our explicit PinRejected check fires.
        // Either failure mode satisfies the security invariant.
        Err(other) => {
            eprintln!("wrong-PIN path failed with {:?} (acceptable)", other);
        }
        Ok(_) => panic!("wrong PIN must not succeed"),
    }
}
