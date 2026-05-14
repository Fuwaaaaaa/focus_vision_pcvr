//! Demo-mode status synthesizer.
//!
//! When the companion is launched with `--demo`, we bypass `status.json` and
//! generate animated state directly from elapsed wall-clock time. This lets
//! reviewers, screen-recorders, and onboarding flows exercise every tab and
//! every connection state without a SteamVR install, an NVIDIA GPU, or a
//! Focus Vision headset.
//!
//! The cycle is 60 s long:
//!
//! - `0..3 s`: Disconnected (engine "warming up")
//! - `3..10 s`: WaitingForPin — fixed PIN "847251" for muscle memory
//! - `10..60 s`: Connected, stats animated with bounded sine waves so the
//!   sparklines actually look alive
//!
//! Past 60 s the synthesizer wraps via `elapsed % 60` so the loop runs
//! indefinitely.

use std::f32::consts::TAU;
use std::time::Duration;

use crate::status_parser::{ConnectionStatus, ParsedStatus, Subsystems};
use fvp_common::STATUS_SCHEMA_VERSION;

const CYCLE_SECS: u64 = 60;
const PHASE_DISCONNECTED_END: u64 = 3;
const PHASE_PIN_END: u64 = 10;
const DEMO_PIN: &str = "847251";
const PIN_LIFETIME_SECONDS: u32 = 300;

/// Produce a `ParsedStatus` for the given elapsed time. Stable for any
/// `elapsed` — wraps via modulo so the caller doesn't have to track ticks.
pub fn synthesize(elapsed: Duration) -> ParsedStatus {
    let total_secs = elapsed.as_secs_f32();
    let cycle_pos = total_secs.rem_euclid(CYCLE_SECS as f32);
    let phase_secs = cycle_pos.floor() as u64;

    let mut out = ParsedStatus {
        schema_version: Some(STATUS_SCHEMA_VERSION),
        ..ParsedStatus::default()
    };

    if phase_secs < PHASE_DISCONNECTED_END {
        out.connection = ConnectionStatus::Disconnected;
        out.pin = "------".to_string();
    } else if phase_secs < PHASE_PIN_END {
        out.connection = ConnectionStatus::WaitingForPin;
        out.pin = DEMO_PIN.to_string();
        // Countdown from PIN_LIFETIME within the 7-second PIN window — gives
        // the countdown UI something to tick down visibly.
        let pin_elapsed = cycle_pos - PHASE_DISCONNECTED_END as f32;
        let pin_window = (PHASE_PIN_END - PHASE_DISCONNECTED_END) as f32;
        let pct_left = 1.0 - (pin_elapsed / pin_window).clamp(0.0, 1.0);
        out.pin_expires_in_seconds = Some((PIN_LIFETIME_SECONDS as f32 * pct_left) as u32);
    } else {
        out.connection = ConnectionStatus::Connected;
        out.pin = DEMO_PIN.to_string();
        out.latency_ms = 13.0 + 4.0 * (TAU * cycle_pos / 8.0).sin();
        out.fps = 90;
        out.bitrate_mbps = 78.0 + 5.0 * (TAU * cycle_pos / 12.0).sin();
        out.subsystems = Subsystems {
            ft_active: Some(true),
            sleep_active: Some(false),
            audio_enabled: Some(true),
            packet_loss_pct: Some(0.3),
        };
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesize_starts_disconnected() {
        let s = synthesize(Duration::from_millis(500));
        assert_eq!(s.connection, ConnectionStatus::Disconnected);
        assert_eq!(s.pin, "------");
    }

    #[test]
    fn synthesize_enters_waiting_for_pin() {
        let s = synthesize(Duration::from_secs(5));
        assert_eq!(s.connection, ConnectionStatus::WaitingForPin);
        assert_eq!(s.pin, DEMO_PIN);
        assert!(s.pin_expires_in_seconds.is_some());
        let remaining = s.pin_expires_in_seconds.unwrap();
        assert!(remaining <= PIN_LIFETIME_SECONDS);
    }

    #[test]
    fn synthesize_enters_connected_with_bounded_stats() {
        let s = synthesize(Duration::from_secs(20));
        assert_eq!(s.connection, ConnectionStatus::Connected);
        assert_eq!(s.fps, 90);
        assert!(s.latency_ms >= 9.0 && s.latency_ms <= 17.0);
        assert!(s.bitrate_mbps >= 73.0 && s.bitrate_mbps <= 83.0);
        assert_eq!(s.subsystems.ft_active, Some(true));
        assert_eq!(s.subsystems.audio_enabled, Some(true));
        assert_eq!(s.subsystems.sleep_active, Some(false));
    }

    #[test]
    fn synthesize_loops_after_one_cycle() {
        let early = synthesize(Duration::from_secs(1));
        let wrapped = synthesize(Duration::from_secs(CYCLE_SECS + 1));
        // Phase identity must hold across the wrap.
        assert_eq!(early.connection, wrapped.connection);
        assert_eq!(early.pin, wrapped.pin);
    }

    #[test]
    fn synthesize_pin_format_is_six_digits() {
        let s = synthesize(Duration::from_secs(5));
        assert_eq!(s.pin.len(), 6);
        assert!(s.pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn synthesize_emits_current_schema_version() {
        let s = synthesize(Duration::from_secs(20));
        assert_eq!(s.schema_version, Some(STATUS_SCHEMA_VERSION));
    }
}
