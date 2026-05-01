//! Thermal governor — caps adaptive bitrate ceiling when GPU temperature
//! crosses staged thresholds, then ramps back over `recovery_seconds`.
//!
//! The governor is source-agnostic: production builds wrap NVML behind
//! `GpuThermalSource`, tests use `MockGpuThermalSource`. NVML wiring is
//! intentionally a stub until Phase 4.3 (real-hardware verification);
//! `try_create_nvml()` always returns `None` for now so call sites cleanly
//! degrade to "thermal disabled" without a hard build dependency.

use std::sync::{atomic::{AtomicU32, Ordering}, Arc};
use std::time::{Duration, Instant};

use crate::config::ThermalConfig;

const LIMIT_MULTIPLIER: f64 = 0.70;
const EMERGENCY_MULTIPLIER: f64 = 0.50;

/// GPU temperature source. Implementations may return `None` when the reading
/// is temporarily unavailable (driver hiccup, NVML init failure) — the governor
/// treats that as "no cap" rather than a hard error.
pub trait GpuThermalSource: Send + Sync {
    fn temperature_celsius(&self) -> Option<u32>;
}

/// Test-only thermal source backed by an atomic counter. Cloning the handle
/// returns a second `Arc` that mutates the same underlying value, so a test
/// can hand the source to the governor while keeping a side channel to vary
/// the simulated temperature.
#[derive(Clone)]
pub struct MockGpuThermalSource {
    temp: Arc<AtomicU32>,
}

impl MockGpuThermalSource {
    pub fn new(initial_celsius: u32) -> Self {
        Self { temp: Arc::new(AtomicU32::new(initial_celsius)) }
    }

    pub fn set_temperature(&self, celsius: u32) {
        self.temp.store(celsius, Ordering::Relaxed);
    }

    /// Returns a second handle pointing at the same underlying atomic. Use
    /// this when a test needs to mutate the source after handing ownership
    /// of the original to a governor.
    pub fn clone_handle(&self) -> Self {
        Self { temp: Arc::clone(&self.temp) }
    }
}

impl GpuThermalSource for MockGpuThermalSource {
    fn temperature_celsius(&self) -> Option<u32> {
        Some(self.temp.load(Ordering::Relaxed))
    }
}

/// Active recovery ramp state. Captured the moment temperature drops below
/// `warn_celsius` while a cap is in effect.
struct Recovery {
    started_at: Instant,
    start_multiplier: f64,
}

pub struct ThermalGovernor {
    config: ThermalConfig,
    source: Box<dyn GpuThermalSource>,
    last_poll: Option<Instant>,
    last_multiplier: f64,
    recovery: Option<Recovery>,
}

impl ThermalGovernor {
    pub fn new(config: ThermalConfig, source: Box<dyn GpuThermalSource>) -> Self {
        Self {
            config,
            source,
            last_poll: None,
            last_multiplier: 1.0,
            recovery: None,
        }
    }

    /// Runtime entry point. Polls the source if `poll_interval_seconds` has
    /// elapsed and returns the current ceiling multiplier (1.0 = no cap).
    /// Returns `None` when the poll was skipped — caller should hold the
    /// previous multiplier (queryable via `current_multiplier`).
    pub fn tick(&mut self) -> Option<f64> {
        self.tick_at(Instant::now())
    }

    /// Test seam: drives the governor with a caller-supplied clock so
    /// recovery ramps can be exercised without `thread::sleep`.
    pub fn tick_at(&mut self, now: Instant) -> Option<f64> {
        if !self.config.enabled {
            self.last_multiplier = 1.0;
            self.recovery = None;
            return Some(1.0);
        }

        if let Some(prev) = self.last_poll {
            let elapsed = now.saturating_duration_since(prev);
            if elapsed < Duration::from_secs(self.config.poll_interval_seconds as u64) {
                return None;
            }
        }
        self.last_poll = Some(now);

        let Some(temp) = self.source.temperature_celsius() else {
            // Reading unavailable — degrade gracefully to "no cap" but do not
            // discard an in-flight recovery state; if the source comes back
            // we'll resume from the same start multiplier.
            self.last_multiplier = 1.0;
            return Some(1.0);
        };

        let cfg = &self.config;
        let multiplier = if temp >= cfg.emergency_celsius {
            self.recovery = None;
            log::warn!("Thermal EMERGENCY: GPU {}°C >= {}°C, capping bitrate to {:.0}%",
                temp, cfg.emergency_celsius, EMERGENCY_MULTIPLIER * 100.0);
            EMERGENCY_MULTIPLIER
        } else if temp >= cfg.limit_celsius {
            self.recovery = None;
            log::warn!("Thermal LIMIT: GPU {}°C >= {}°C, capping bitrate to {:.0}%",
                temp, cfg.limit_celsius, LIMIT_MULTIPLIER * 100.0);
            LIMIT_MULTIPLIER
        } else if temp >= cfg.warn_celsius {
            // Warn zone: log but hold whatever cap is currently in force.
            // Recovery does not start until temp drops below `warn_celsius`.
            self.recovery = None;
            log::info!("Thermal WARN: GPU {}°C >= {}°C", temp, cfg.warn_celsius);
            self.last_multiplier
        } else {
            // Below warn. If we were capped, begin (or continue) the ramp.
            if self.last_multiplier < 1.0 && self.recovery.is_none() {
                self.recovery = Some(Recovery {
                    started_at: now,
                    start_multiplier: self.last_multiplier,
                });
                log::info!("Thermal recovery started from {:.2}", self.last_multiplier);
            }
            match &self.recovery {
                Some(r) => {
                    let elapsed = now.saturating_duration_since(r.started_at);
                    let progress = (elapsed.as_secs_f64()
                        / cfg.recovery_seconds.max(1) as f64)
                        .clamp(0.0, 1.0);
                    let m = r.start_multiplier + (1.0 - r.start_multiplier) * progress;
                    if m >= 1.0 - f64::EPSILON {
                        self.recovery = None;
                        1.0
                    } else {
                        m
                    }
                }
                None => 1.0,
            }
        };

        self.last_multiplier = multiplier;
        Some(multiplier)
    }

    /// Latest computed multiplier — useful when the caller polls more often
    /// than `poll_interval_seconds` and just wants the cached value.
    pub fn current_multiplier(&self) -> f64 {
        self.last_multiplier
    }

    /// Apply the current ceiling to a base bitrate (in bps).
    pub fn apply_ceiling(&self, base_bitrate_bps: u64) -> u64 {
        ((base_bitrate_bps as f64) * self.last_multiplier) as u64
    }

    /// Stub for NVML-backed construction. Returns `None` until Phase 4.3
    /// adds the `nvml-wrapper` dependency and real hardware verification.
    /// Call sites must treat `None` as "thermal control unavailable" and
    /// continue without a cap.
    pub fn try_create_nvml(_config: ThermalConfig) -> Option<Self> {
        log::debug!("NVML thermal source not yet wired — Phase 4.3 deliverable");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThermalConfig;
    use std::time::{Duration, Instant};

    fn cfg() -> ThermalConfig {
        ThermalConfig {
            enabled: true,
            poll_interval_seconds: 5,
            warn_celsius: 75,
            limit_celsius: 85,
            emergency_celsius: 90,
            recovery_seconds: 30,
        }
    }

    #[test]
    fn test_mock_source_returns_set_value() {
        let src = MockGpuThermalSource::new(60);
        assert_eq!(src.temperature_celsius(), Some(60));
        src.set_temperature(80);
        assert_eq!(src.temperature_celsius(), Some(80));
    }

    #[test]
    fn test_governor_below_warn_returns_full_ceiling() {
        let src = Box::new(MockGpuThermalSource::new(50));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let m = gov.tick_at(Instant::now()).unwrap();
        assert!((m - 1.0).abs() < f64::EPSILON, "below warn → full ceiling");
    }

    #[test]
    fn test_governor_warn_zone_logs_no_action() {
        // 75 (== warn) up to but not including limit (85): no bitrate cap.
        let src = Box::new(MockGpuThermalSource::new(80));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let m = gov.tick_at(Instant::now()).unwrap();
        assert!((m - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_governor_limit_zone_returns_70_percent() {
        let src = Box::new(MockGpuThermalSource::new(85));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let m = gov.tick_at(Instant::now()).unwrap();
        assert!((m - 0.70).abs() < 1e-6, "limit → 70% ceiling, got {}", m);
    }

    #[test]
    fn test_governor_emergency_zone_returns_50_percent() {
        let src = Box::new(MockGpuThermalSource::new(95));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let m = gov.tick_at(Instant::now()).unwrap();
        assert!((m - 0.50).abs() < 1e-6, "emergency → 50% ceiling, got {}", m);
    }

    #[test]
    fn test_governor_recovery_progressive_within_window() {
        let src = MockGpuThermalSource::new(95);
        let src_handle = src.clone_handle();
        let mut gov = ThermalGovernor::new(cfg(), Box::new(src));
        let t0 = Instant::now();

        // Hit emergency.
        let m_hot = gov.tick_at(t0).unwrap();
        assert!((m_hot - 0.50).abs() < 1e-6);

        // Cool below warn — recovery should start.
        src_handle.set_temperature(60);
        let _ = gov.tick_at(t0 + Duration::from_secs(6));

        // Halfway through recovery: linear interpolation between 0.5 and 1.0.
        let m_mid = gov.tick_at(t0 + Duration::from_secs(6 + 15)).unwrap();
        assert!(m_mid > 0.50 && m_mid < 1.0, "mid recovery, got {}", m_mid);
        assert!((m_mid - 0.75).abs() < 0.05, "expected ~0.75, got {}", m_mid);
    }

    #[test]
    fn test_governor_recovery_completes_after_window() {
        let src = MockGpuThermalSource::new(95);
        let src_handle = src.clone_handle();
        let mut gov = ThermalGovernor::new(cfg(), Box::new(src));
        let t0 = Instant::now();

        let _ = gov.tick_at(t0); // hot
        src_handle.set_temperature(60);
        let _ = gov.tick_at(t0 + Duration::from_secs(6));

        // After full recovery window, multiplier returns to 1.0.
        let m = gov.tick_at(t0 + Duration::from_secs(6 + 31)).unwrap();
        assert!((m - 1.0).abs() < 1e-6, "fully recovered, got {}", m);
    }

    #[test]
    fn test_governor_disabled_always_returns_full() {
        let mut c = cfg();
        c.enabled = false;
        let src = Box::new(MockGpuThermalSource::new(95));
        let mut gov = ThermalGovernor::new(c, src);
        let m = gov.tick_at(Instant::now()).unwrap();
        assert!((m - 1.0).abs() < f64::EPSILON, "disabled → no cap");
    }

    #[test]
    fn test_governor_source_unavailable_treats_as_full() {
        let src = Box::new(UnavailableSource);
        let mut gov = ThermalGovernor::new(cfg(), src);
        let m = gov.tick_at(Instant::now()).unwrap();
        assert!((m - 1.0).abs() < f64::EPSILON, "no reading → no cap");
    }

    #[test]
    fn test_governor_respects_poll_interval() {
        let src = Box::new(MockGpuThermalSource::new(50));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let t0 = Instant::now();
        assert!(gov.tick_at(t0).is_some(), "first tick polls");
        // Within poll interval: skipped.
        assert!(gov.tick_at(t0 + Duration::from_secs(1)).is_none());
        // After poll interval: polls again.
        assert!(gov.tick_at(t0 + Duration::from_secs(6)).is_some());
    }

    #[test]
    fn test_apply_ceiling_clamps_to_multiplier() {
        let src = Box::new(MockGpuThermalSource::new(85));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let _ = gov.tick_at(Instant::now());
        // 100 Mbps * 0.70 cap → 70 Mbps
        let capped = gov.apply_ceiling(100_000_000);
        assert_eq!(capped, 70_000_000);
    }

    #[test]
    fn test_apply_ceiling_passthrough_when_cool() {
        let src = Box::new(MockGpuThermalSource::new(50));
        let mut gov = ThermalGovernor::new(cfg(), src);
        let _ = gov.tick_at(Instant::now());
        assert_eq!(gov.apply_ceiling(80_000_000), 80_000_000);
    }

    #[test]
    fn test_try_create_nvml_returns_none_until_phase4() {
        // NVML wiring is stubbed; production callers must gracefully
        // disable thermal control when this returns None.
        assert!(ThermalGovernor::try_create_nvml(cfg()).is_none());
    }

    /// A source that always reports "no reading" — simulates NVML init
    /// failure or unsupported hardware.
    struct UnavailableSource;
    impl GpuThermalSource for UnavailableSource {
        fn temperature_celsius(&self) -> Option<u32> { None }
    }
}
