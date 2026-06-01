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

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use fvp_common::protocol::VideoCodec;
use streaming_engine::config::AppConfig;
use streaming_engine::engine::{EncodedFrame, StreamingEngine};
use streaming_engine::metrics::latency::FrameTimestamps;
use streaming_engine::simulator::{run as run_mock_client, MockClientConfig};
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

#[test]
fn headless_e2e_basic_video_flow() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .is_test(true)
        .try_init();

    // 1. Pick ports, clear stale PIN, configure engine, launch.
    delete_stale_status();
    let (tcp_port, udp_port) = pick_free_ports();
    let mut config = AppConfig::default();
    config.network.tcp_port = tcp_port;
    config.network.udp_port = udp_port;
    // Use a small framerate so the test is bounded and the channel
    // doesn't spam frame drops while waiting for the client.
    config.video.framerate = 60;

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
    while start.elapsed() < Duration::from_millis(2200) {
        let synth = stream.next_frame();
        let frame = EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: synth.is_idr,
            timestamps: FrameTimestamps::new(synth.frame_index),
        };
        frames_offered += 1;
        if engine.submit_frame(frame) {
            frames_accepted += 1;
        }
        next_tick += frame_period;
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }

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
    assert!(frames_accepted > 0,
        "engine should accept some submitted frames once the channel drains");
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
    let mut config = AppConfig::default();
    config.network.tcp_port = tcp_port;
    config.network.udp_port = udp_port;
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
