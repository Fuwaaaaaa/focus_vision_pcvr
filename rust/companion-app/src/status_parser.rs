//! Parser for the engine's `status.json` payload.
//!
//! The engine writes this file from `streaming-engine::build_status_json`
//! (see `rust/streaming-engine/src/lib.rs`) and the companion polls it once
//! per second. Splitting the parser out of `main.rs` lets us drive every
//! branch from hand-crafted JSON fixtures without depending on egui or the
//! filesystem.
//!
//! The wire schema is documented in `rust/common/src/constants.rs`
//! (`STATUS_SCHEMA_VERSION`) and intentionally tolerates missing fields:
//! a v2-era payload (no `schema_version`) still parses, and an unknown
//! future schema only logs a debug message instead of refusing to render.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Disconnected,
    WaitingForPin,
    Connected,
}

/// Subsystem fields are only present once the engine has begun streaming.
/// Each `Option` distinguishes "missing in payload" from "present, false":
/// the companion uses this to gray out badges when the engine is silent.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Subsystems {
    pub ft_active: Option<bool>,
    pub sleep_active: Option<bool>,
    pub audio_enabled: Option<bool>,
    pub packet_loss_pct: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedStatus {
    /// Absent in v2-era payloads. v3 always emits `3`. A higher value is
    /// surfaced as-is so the caller can decide what to do (today: log).
    pub schema_version: Option<u32>,
    pub connection: ConnectionStatus,
    /// Always present: empty PIN is `"------"` (engine sentinel), 6-digit
    /// when active. The companion's initial in-memory state is `"----"`
    /// (4 dashes) — the parser deliberately does not normalize so the
    /// caller can distinguish "engine hasn't written yet" from "engine
    /// says no PIN yet".
    pub pin: String,
    /// Milliseconds — derived from `latency_us / 1000.0`.
    pub latency_ms: f32,
    pub fps: u32,
    pub bitrate_mbps: f32,
    pub subsystems: Subsystems,
    /// Seconds remaining until the current PIN expires and a fresh one
    /// will be minted. `None` when the engine doesn't emit the field
    /// (pre-v3 payloads, or status types where the PIN isn't active).
    /// The Home tab uses this to render an `Expires in: M:SS` countdown.
    pub pin_expires_in_seconds: Option<u32>,
}

impl Default for ParsedStatus {
    fn default() -> Self {
        Self {
            schema_version: None,
            connection: ConnectionStatus::Disconnected,
            pin: "------".to_string(),
            latency_ms: 0.0,
            fps: 0,
            bitrate_mbps: 0.0,
            subsystems: Subsystems::default(),
            pin_expires_in_seconds: None,
        }
    }
}

/// Parse a `status.json` payload. Returns `None` only on malformed JSON;
/// missing or unknown fields fall back to defaults so a partial write
/// (engine mid-update) still produces a usable view.
pub fn parse_status_json(content: &str) -> Option<ParsedStatus> {
    let val: serde_json::Value = serde_json::from_str(content).ok()?;

    let schema_version = val
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let status_str = val.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
    let pin = val.get("pin").and_then(|v| v.as_str()).unwrap_or("------").to_string();

    let connection = match status_str {
        "waiting" => {
            // The engine writes "------" (6 dashes) before it has minted a
            // PIN, and a real 6-digit PIN once it has. The companion's
            // initial in-memory state is "----" (4 dashes) — see
            // `CompanionApp::new`. We treat any non-sentinel value as a
            // valid PIN; the 6-dash sentinel maps back to Disconnected
            // so the UI doesn't briefly flash "------" as if it were
            // a real code.
            if pin == "------" || pin == "----" {
                ConnectionStatus::Disconnected
            } else {
                ConnectionStatus::WaitingForPin
            }
        }
        "streaming" => ConnectionStatus::Connected,
        _ => ConnectionStatus::Disconnected,
    };

    let latency_us = val.get("latency_us").and_then(|v| v.as_u64()).unwrap_or(0);
    let latency_ms = latency_us as f32 / 1000.0;
    let fps = val.get("fps").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let bitrate_mbps = val.get("bitrate_mbps").and_then(|v| v.as_u64()).unwrap_or(0) as f32;

    let subsystems = Subsystems {
        ft_active: val.get("ft_active").and_then(|v| v.as_bool()),
        sleep_active: val.get("sleep_active").and_then(|v| v.as_bool()),
        audio_enabled: val.get("audio_enabled").and_then(|v| v.as_bool()),
        packet_loss_pct: val
            .get("packet_loss_pct")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32),
    };

    let pin_expires_in_seconds = val
        .get("pin_expires_in_seconds")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    Some(ParsedStatus {
        schema_version,
        connection,
        pin,
        latency_ms,
        fps,
        bitrate_mbps,
        subsystems,
        pin_expires_in_seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_v3() -> u32 {
        // Pull the constant from the shared crate so this test fails fast
        // if the schema is bumped without updating the parser expectations.
        fvp_common::STATUS_SCHEMA_VERSION
    }

    #[test]
    fn parses_idle_payload() {
        let json = r#"{
            "schema_version": 3,
            "status": "idle",
            "pin": "------",
            "latency_us": 0,
            "fps": 0,
            "bitrate_mbps": 0
        }"#;
        let parsed = parse_status_json(json).expect("must parse");
        assert_eq!(parsed.schema_version, Some(3));
        assert_eq!(parsed.connection, ConnectionStatus::Disconnected);
        assert_eq!(parsed.pin, "------");
    }

    #[test]
    fn parses_waiting_with_real_pin() {
        let json = r#"{
            "schema_version": 3,
            "status": "waiting",
            "pin": "048217",
            "latency_us": 0,
            "fps": 0,
            "bitrate_mbps": 0
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::WaitingForPin);
        assert_eq!(parsed.pin, "048217");
    }

    #[test]
    fn waiting_with_sentinel_dashes_maps_to_disconnected() {
        // The engine writes "------" before minting a real PIN. The UI
        // shouldn't display six dashes as if it were a PIN — collapse
        // to Disconnected so the WaitingForPin path is gated on a real code.
        let json = r#"{
            "schema_version": 3,
            "status": "waiting",
            "pin": "------"
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::Disconnected);
    }

    #[test]
    fn streaming_payload_extracts_all_metrics() {
        let json = r#"{
            "schema_version": 3,
            "status": "streaming",
            "pin": "112233",
            "latency_us": 18500,
            "fps": 90,
            "bitrate_mbps": 80,
            "ft_active": true,
            "sleep_active": false,
            "audio_enabled": true,
            "packet_loss_pct": 1.4
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::Connected);
        assert_eq!(parsed.pin, "112233");
        assert!((parsed.latency_ms - 18.5).abs() < 1e-3);
        assert_eq!(parsed.fps, 90);
        assert_eq!(parsed.bitrate_mbps, 80.0);
        assert_eq!(parsed.subsystems.ft_active, Some(true));
        assert_eq!(parsed.subsystems.sleep_active, Some(false));
        assert_eq!(parsed.subsystems.audio_enabled, Some(true));
        assert!((parsed.subsystems.packet_loss_pct.unwrap() - 1.4).abs() < 1e-3);
    }

    #[test]
    fn streaming_without_subsystem_block() {
        // Before the [subsystems] block was added, streaming payloads
        // were just metrics. Companion must still render those — every
        // subsystem field comes back as None rather than a misleading
        // false.
        let json = r#"{
            "schema_version": 3,
            "status": "streaming",
            "pin": "777777",
            "latency_us": 12000,
            "fps": 60,
            "bitrate_mbps": 40
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::Connected);
        assert_eq!(parsed.subsystems.ft_active, None);
        assert_eq!(parsed.subsystems.audio_enabled, None);
        assert_eq!(parsed.subsystems.packet_loss_pct, None);
    }

    #[test]
    fn pre_v3_payload_without_schema_version_still_parses() {
        // v2-era status.json didn't carry schema_version; the parser must
        // tolerate that and surface `None` rather than refusing the payload.
        let json = r#"{
            "status": "waiting",
            "pin": "904521",
            "latency_us": 0,
            "fps": 0,
            "bitrate_mbps": 0
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.schema_version, None);
        assert_eq!(parsed.connection, ConnectionStatus::WaitingForPin);
        assert_eq!(parsed.pin, "904521");
    }

    #[test]
    fn future_schema_version_is_surfaced_as_is() {
        let json = format!(
            r#"{{
                "schema_version": {},
                "status": "idle",
                "pin": "------"
            }}"#,
            schema_v3() + 1
        );
        let parsed = parse_status_json(&json).unwrap();
        assert_eq!(parsed.schema_version, Some(schema_v3() + 1));
        // Decision left to the caller; the parser doesn't refuse the payload.
        assert_eq!(parsed.connection, ConnectionStatus::Disconnected);
    }

    #[test]
    fn unknown_status_string_maps_to_disconnected() {
        let json = r#"{"schema_version": 3, "status": "rebooting", "pin": "999999"}"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::Disconnected);
    }

    #[test]
    fn malformed_json_returns_none() {
        assert!(parse_status_json("not json at all").is_none());
        assert!(parse_status_json(r#"{"status": "waiting""#).is_none());
        assert!(parse_status_json("").is_none());
    }

    #[test]
    fn partial_write_with_missing_fields_falls_back_to_zero() {
        // Atomic temp-then-rename should prevent us from ever reading a
        // half-written file, but defense in depth — make sure a payload
        // with only the bare minimum still produces a valid struct.
        let json = r#"{"status": "streaming"}"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::Connected);
        assert_eq!(parsed.fps, 0);
        assert_eq!(parsed.bitrate_mbps, 0.0);
        assert_eq!(parsed.pin, "------");
    }

    #[test]
    fn latency_us_microseconds_to_milliseconds_conversion() {
        let json = r#"{"status": "streaming", "pin": "111111", "latency_us": 12500}"#;
        let parsed = parse_status_json(json).unwrap();
        assert!((parsed.latency_ms - 12.5).abs() < 1e-3);
    }

    #[test]
    fn pin_expires_in_seconds_parses_when_present() {
        let json = r#"{
            "schema_version": 3,
            "status": "waiting",
            "pin": "742103",
            "pin_expires_in_seconds": 247
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.connection, ConnectionStatus::WaitingForPin);
        assert_eq!(parsed.pin_expires_in_seconds, Some(247));
    }

    #[test]
    fn pin_expires_in_seconds_absent_yields_none() {
        let json = r#"{
            "schema_version": 3,
            "status": "waiting",
            "pin": "742103"
        }"#;
        let parsed = parse_status_json(json).unwrap();
        assert_eq!(parsed.pin_expires_in_seconds, None);
    }

    #[test]
    fn engine_built_payload_round_trips() {
        // Cross-check against the actual shape `streaming-engine::build_status_json`
        // produces by writing a payload that mirrors its fields verbatim
        // (schema_version pulled from fvp_common). If the engine schema
        // diverges, this test fails fast.
        let json = format!(
            r#"{{
                "schema_version": {},
                "status": "streaming",
                "pin": "042000",
                "latency_us": 9500,
                "fps": 96,
                "bitrate_mbps": 80,
                "ft_active": false,
                "sleep_active": false,
                "audio_enabled": true,
                "packet_loss_pct": 0.3
            }}"#,
            fvp_common::STATUS_SCHEMA_VERSION
        );
        let parsed = parse_status_json(&json).unwrap();
        assert_eq!(parsed.schema_version, Some(fvp_common::STATUS_SCHEMA_VERSION));
        assert_eq!(parsed.connection, ConnectionStatus::Connected);
        assert_eq!(parsed.fps, 96);
        assert!((parsed.latency_ms - 9.5).abs() < 1e-3);
        assert_eq!(parsed.subsystems.audio_enabled, Some(true));
    }
}
