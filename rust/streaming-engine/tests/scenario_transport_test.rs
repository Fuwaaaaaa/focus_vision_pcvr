//! Transport-layer scenario tests: packet loss simulation and session
//! recording. Kept in a separate test binary from `scenario_test.rs` so
//! the Windows resource teardown that one binary's repeated engine
//! lifecycle accumulates doesn't bleed into the strict video-flow
//! assertions here. Each `#[test]` runs in its own cargo test binary
//! process, which is the simplest isolation cargo offers.

#![cfg(feature = "simulator")]

use std::path::PathBuf;

use streaming_engine::simulator::scenario::{parse_scenario_file, run_scenario, Scenario};

fn scenarios_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("scenarios")
}

fn load(name: &str) -> Scenario {
    let path = scenarios_dir().join(name);
    parse_scenario_file(&path)
        .unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e))
}

#[test]
fn scenario_packet_loss() {
    let scenario = load("packet_loss.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "packet_loss: passed={} duration={:?} pkts={} frames={}",
            report.passed,
            report.duration,
            stats.video_packets_received,
            stats.frames_decoded,
        );
    }
    report.assert_passed();
}

#[test]
fn scenario_recording() {
    let scenario = load("recording.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "recording: passed={} duration={:?} frames={} pkts={}",
            report.passed,
            report.duration,
            stats.frames_decoded,
            stats.video_packets_received,
        );
    }
    report.assert_passed();
}

/// Run both H.264 and H.265 scenarios back-to-back and emit a side-by-side
/// summary of decode-latency percentiles. The assertion gates are loose
/// (50 ms p99 ceiling) — the goal isn't to pick a winner but to surface
/// the synthetic depacketization profile so a regression in the codec path
/// produces a numeric diff in CI logs that humans can compare to the
/// previous release's run.
///
/// Real-world MediaCodec decode latency comparison still needs the HMD —
/// this scenario only measures the engine + transport + depacketize path,
/// which is what changes when codec switches affect packetization shape
/// (HEVC has larger IDR frames → more FEC shards → slightly more
/// depacketize work even with identical synthetic NAL sizes).
#[test]
fn scenario_codec_comparison() {
    let h264 = load("codec_comparison_h264.json");
    let h264_report = run_scenario(&h264)
        .unwrap_or_else(|e| panic!("h264 scenario runner failed: {}", e));
    let h264_stats = h264_report
        .stats
        .clone()
        .expect("h264 scenario should produce stats");
    h264_report.assert_passed();

    let h265 = load("codec_comparison_h265.json");
    let h265_report = run_scenario(&h265)
        .unwrap_or_else(|e| panic!("h265 scenario runner failed: {}", e));
    let h265_stats = h265_report
        .stats
        .clone()
        .expect("h265 scenario should produce stats");
    h265_report.assert_passed();

    eprintln!("==================== CODEC COMPARISON ====================");
    eprintln!(
        "                    H.264                H.265           diff (H265-H264)"
    );
    eprintln!(
        "decode p50 (us)  {:>10}        {:>10}        {:>+10}",
        h264_stats.depacketize_latency_us_p50,
        h265_stats.depacketize_latency_us_p50,
        h265_stats.depacketize_latency_us_p50 as i64
            - h264_stats.depacketize_latency_us_p50 as i64
    );
    eprintln!(
        "decode p95 (us)  {:>10}        {:>10}        {:>+10}",
        h264_stats.depacketize_latency_us_p95,
        h265_stats.depacketize_latency_us_p95,
        h265_stats.depacketize_latency_us_p95 as i64
            - h264_stats.depacketize_latency_us_p95 as i64
    );
    eprintln!(
        "decode p99 (us)  {:>10}        {:>10}        {:>+10}",
        h264_stats.depacketize_latency_us_p99,
        h265_stats.depacketize_latency_us_p99,
        h265_stats.depacketize_latency_us_p99 as i64
            - h264_stats.depacketize_latency_us_p99 as i64
    );
    eprintln!(
        "samples          {:>10}        {:>10}",
        h264_stats.depacketize_samples_count, h265_stats.depacketize_samples_count
    );
    eprintln!(
        "frames           {:>10}        {:>10}",
        h264_stats.frames_decoded, h265_stats.frames_decoded
    );
    eprintln!(
        "packets          {:>10}        {:>10}",
        h264_stats.video_packets_received, h265_stats.video_packets_received
    );
    eprintln!("==========================================================");
}

#[test]
fn scenario_face_tracking_blink() {
    let scenario = load("face_tracking_patterns.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "face_tracking_patterns: passed={} duration={:?} face_msgs={} osc={}",
            report.passed,
            report.duration,
            stats.face_messages_sent,
            stats.osc_messages.len(),
        );
    }
    report.assert_passed();
}

#[test]
fn scenario_face_tracking_talk() {
    let scenario = load("face_tracking_talk.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    report.assert_passed();
}

#[test]
fn scenario_face_tracking_smile() {
    let scenario = load("face_tracking_smile.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    report.assert_passed();
}

#[test]
fn scenario_frame_jitter() {
    let scenario = load("frame_jitter.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "frame_jitter: passed={} duration={:?} frames={} pkts={}",
            report.passed,
            report.duration,
            stats.frames_decoded,
            stats.video_packets_received,
        );
    }
    report.assert_passed();
}

/// Long-duration stability run. Excluded from default CI by `#[ignore]`;
/// invoke explicitly with `cargo test -- --ignored long_run_stability`.
/// CI fires this from a nightly schedule (see `.github/workflows/build.yml`).
#[test]
#[ignore]
fn long_run_stability() {
    let scenario = load("long_run_stability.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("long_run scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "long_run_stability: passed={} duration={:?} frames={} pkts={} hb={}",
            report.passed,
            report.duration,
            stats.frames_decoded,
            stats.video_packets_received,
            stats.heartbeats_sent,
        );
    }
    report.assert_passed();
}
