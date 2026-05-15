//! Scenario-driven end-to-end regression tests.
//!
//! Each `#[test]` here loads a JSON scenario from `tests/scenarios/`, drives
//! the engine + mock client (+ optional tracking sender) via the runner in
//! `streaming_engine::simulator::scenario`, and asserts that all checks in
//! the scenario's `assertions` block pass.
//!
//! Must run with `--test-threads=1` because each scenario claims the
//! process-global `%APPDATA%/FocusVisionPCVR/status.json` for PIN discovery.

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
fn scenario_0_golden_path() {
    let scenario = load("golden_path.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    eprintln!(
        "golden_path: passed={} duration={:?} stats={:?}",
        report.passed, report.duration, report.stats
    );
    report.assert_passed();
}

#[test]
fn scenario_sleep_cycle() {
    let scenario = load("sleep_cycle.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "sleep_cycle: passed={} duration={:?} frames={} sleep_enter={} sleep_exit={}",
            report.passed,
            report.duration,
            stats.frames_decoded,
            stats.sleep_enter_count,
            stats.sleep_exit_count,
        );
    }
    report.assert_passed();
}

#[test]
fn scenario_haptic() {
    let scenario = load("haptic.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "haptic: passed={} duration={:?} frames={} haptic_received={} hb={}",
            report.passed,
            report.duration,
            stats.frames_decoded,
            stats.haptic_events_received.len(),
            stats.heartbeats_sent,
        );
    }
    report.assert_passed();
}

// NOTE: Test name prefixed with "z_" so it runs AFTER `scenario_golden_path`
// (and `scenario_haptic`) in cargo test's default alphabetical order. With
// the order reversed, some Windows-specific state left by the OSC bridge +
// FACE_DATA path — likely a lingering UDP socket or WASAPI device handle —
// prevents the next scenario's video pipeline from delivering RTP packets
// to the mock client. The root cause is tracked separately; for now we
// sequence the scenarios so they coexist cleanly in a single test process.
// Process isolation (e.g. cargo nextest) would also avoid this.
#[test]
fn scenario_z_face_tracking() {
    let scenario = load("face_tracking.json");
    let report = run_scenario(&scenario)
        .unwrap_or_else(|e| panic!("scenario runner failed: {}", e));
    if let Some(stats) = &report.stats {
        eprintln!(
            "face_tracking: passed={} duration={:?} frames={} idr={} pkts={} hb={} face_sent={} osc_keys={} osc_total={}",
            report.passed,
            report.duration,
            stats.frames_decoded,
            stats.idr_frames_seen,
            stats.video_packets_received,
            stats.heartbeats_sent,
            stats.face_messages_sent,
            stats.osc_messages.len(),
            stats.osc_messages.values().map(|v| v.len()).sum::<usize>(),
        );
    } else {
        eprintln!("face_tracking: passed={} duration={:?} (no stats)", report.passed, report.duration);
    }
    report.assert_passed();
}

#[test]
fn deserialize_all_scenarios() {
    // Type-regression guard: any future scenario file must remain parseable.
    let dir = scenarios_dir();
    let entries = std::fs::read_dir(&dir).expect("read scenarios/");
    let mut count = 0;
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        parse_scenario_file(&path)
            .unwrap_or_else(|e| panic!("scenario {} failed to parse: {}", name, e));
        count += 1;
    }
    assert!(count >= 1, "expected at least one scenario JSON in {:?}", dir);
}
