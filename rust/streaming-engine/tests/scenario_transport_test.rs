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
