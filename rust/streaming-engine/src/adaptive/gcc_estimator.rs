//! GCC-inspired delay-based bandwidth estimator.
//! Tracks one-way delay variation from transport feedback to detect
//! congestion before packet loss occurs.

use std::collections::VecDeque;
use std::time::Instant;

use fvp_common::protocol::TransportFeedbackEntry;

/// State of the delay trend detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayTrend {
    Normal,
    Increasing,
    Overuse,
}

/// GCC-inspired delay-based bandwidth estimator.
pub struct GccEstimator {
    delay_samples: VecDeque<f64>,
    max_samples: usize,
    filtered_gradient: f64,
    overuse_threshold: f64,
    trend: DelayTrend,
    alpha: f64,
    last_recv_delta_us: Option<i32>,
    current_bitrate_bps: u64,
    last_probe: Instant,
    probing: bool,
}

impl GccEstimator {
    pub fn new(initial_bitrate_bps: u64) -> Self {
        Self {
            delay_samples: VecDeque::new(),
            max_samples: 60,
            filtered_gradient: 0.0,
            overuse_threshold: 6.0,
            trend: DelayTrend::Normal,
            alpha: 0.3,
            last_recv_delta_us: None,
            current_bitrate_bps: initial_bitrate_bps,
            last_probe: Instant::now(),
            probing: false,
        }
    }

    /// Process transport feedback entries to compute one-way delay gradient.
    /// Entries contain per-packet receive timestamp deltas from the HMD.
    pub fn process_feedback(&mut self, entries: &[TransportFeedbackEntry]) {
        if entries.len() < 2 {
            return;
        }

        for window in entries.windows(2) {
            let gradient_us = (window[1].recv_delta_us - window[0].recv_delta_us) as f64;
            let gradient_ms = gradient_us / 1000.0;

            if self.last_recv_delta_us.is_some() {
                self.filtered_gradient =
                    self.alpha * gradient_ms + (1.0 - self.alpha) * self.filtered_gradient;
            } else {
                self.filtered_gradient = gradient_ms;
            }

            self.delay_samples.push_back(gradient_ms);
            if self.delay_samples.len() > self.max_samples {
                self.delay_samples.pop_front();
            }
        }

        self.last_recv_delta_us = Some(entries.last().unwrap().recv_delta_us);

        // Adaptive threshold
        self.overuse_threshold =
            0.99 * self.overuse_threshold + 0.01 * self.filtered_gradient.abs().max(6.0);

        // Classify trend
        if self.filtered_gradient > self.overuse_threshold {
            self.trend = DelayTrend::Overuse;
        } else if self.filtered_gradient > self.overuse_threshold * 0.5 {
            self.trend = DelayTrend::Increasing;
        } else {
            self.trend = DelayTrend::Normal;
        }
    }

    /// Current delay trend.
    pub fn trend(&self) -> DelayTrend {
        self.trend
    }

    /// Filtered one-way delay gradient in milliseconds.
    /// Positive = congestion, negative = recovery.
    pub fn delay_gradient_ms(&self) -> f64 {
        self.filtered_gradient
    }

    /// Suggested bitrate multiplier based on current trend.
    pub fn bitrate_multiplier(&self) -> f64 {
        match self.trend {
            DelayTrend::Overuse => 0.85,
            DelayTrend::Increasing => 0.95,
            DelayTrend::Normal => {
                if self.filtered_gradient < -2.0 {
                    1.05
                } else {
                    1.0
                }
            }
        }
    }

    /// Whether conditions are stable enough to probe for more bandwidth.
    /// Normal trend for 30+ seconds with low gradient.
    pub fn should_probe(&self) -> bool {
        self.trend == DelayTrend::Normal
            && self.filtered_gradient.abs() < 2.0
            && self.last_probe.elapsed().as_secs() >= 30
    }

    /// Begin a bandwidth probe phase.
    pub fn start_probe(&mut self) {
        self.probing = true;
        self.last_probe = Instant::now();
    }

    /// End probe. Returns true if the link remained stable during the probe.
    pub fn end_probe(&mut self) -> bool {
        self.probing = false;
        self.trend == DelayTrend::Normal
    }

    /// Update the current bitrate (called after controller adjusts).
    pub fn set_current_bitrate(&mut self, bps: u64) {
        self.current_bitrate_bps = bps;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fvp_common::protocol::TransportFeedbackEntry;

    #[test]
    fn test_initial_state() {
        let gcc = GccEstimator::new(80_000_000);
        assert_eq!(gcc.trend(), DelayTrend::Normal);
        assert_eq!(gcc.delay_gradient_ms(), 0.0);
        assert_eq!(gcc.bitrate_multiplier(), 1.0);
    }

    #[test]
    fn test_stable_link_stays_normal() {
        let mut gcc = GccEstimator::new(80_000_000);
        // Equal inter-arrival deltas → gradient ≈ 0
        let entries: Vec<TransportFeedbackEntry> = (0..10)
            .map(|i| TransportFeedbackEntry {
                sequence: i,
                recv_delta_us: 10_000,
            })
            .collect();
        gcc.process_feedback(&entries);
        assert_eq!(gcc.trend(), DelayTrend::Normal);
        assert!(
            gcc.delay_gradient_ms().abs() < 0.1,
            "Expected gradient ≈ 0, got {}",
            gcc.delay_gradient_ms()
        );
    }

    #[test]
    fn test_increasing_delay_detects_overuse() {
        let mut gcc = GccEstimator::new(80_000_000);
        // Strongly increasing inter-arrival deltas to push past threshold
        for _ in 0..5 {
            let entries = vec![
                TransportFeedbackEntry { sequence: 0, recv_delta_us: 10_000 },
                TransportFeedbackEntry { sequence: 1, recv_delta_us: 15_000 },
                TransportFeedbackEntry { sequence: 2, recv_delta_us: 22_000 },
                TransportFeedbackEntry { sequence: 3, recv_delta_us: 32_000 },
            ];
            gcc.process_feedback(&entries);
        }
        assert!(
            gcc.trend() == DelayTrend::Overuse || gcc.trend() == DelayTrend::Increasing,
            "Expected Overuse or Increasing, got {:?}",
            gcc.trend()
        );
        assert!(
            gcc.delay_gradient_ms() > 0.0,
            "Expected positive gradient, got {}",
            gcc.delay_gradient_ms()
        );
    }

    #[test]
    fn test_decreasing_delay_suggests_increase() {
        let mut gcc = GccEstimator::new(80_000_000);
        // Decreasing inter-arrival deltas → negative gradient
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 20_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 17_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 13_000 },
            TransportFeedbackEntry { sequence: 3, recv_delta_us: 8_000 },
        ];
        gcc.process_feedback(&entries);
        assert!(
            gcc.delay_gradient_ms() < 0.0,
            "Expected negative gradient, got {}",
            gcc.delay_gradient_ms()
        );
        assert_eq!(gcc.trend(), DelayTrend::Normal);
    }

    #[test]
    fn test_single_feedback_no_crash() {
        let mut gcc = GccEstimator::new(80_000_000);
        gcc.process_feedback(&[TransportFeedbackEntry {
            sequence: 0,
            recv_delta_us: 10_000,
        }]);
        assert_eq!(gcc.trend(), DelayTrend::Normal);
        assert_eq!(gcc.delay_gradient_ms(), 0.0);
    }

    #[test]
    fn test_bitrate_multiplier_ranges() {
        let mut gcc = GccEstimator::new(80_000_000);

        // Normal → 1.0
        assert_eq!(gcc.bitrate_multiplier(), 1.0);

        // Feed strongly negative gradient for underuse → 1.05
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 30_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 25_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 18_000 },
            TransportFeedbackEntry { sequence: 3, recv_delta_us: 10_000 },
        ];
        gcc.process_feedback(&entries);
        assert!(gcc.delay_gradient_ms() < -2.0, "gradient should be < -2.0");
        assert_eq!(gcc.bitrate_multiplier(), 1.05);

        // Feed strongly positive gradient repeatedly to reach overuse
        for _ in 0..10 {
            let entries = vec![
                TransportFeedbackEntry { sequence: 0, recv_delta_us: 10_000 },
                TransportFeedbackEntry { sequence: 1, recv_delta_us: 20_000 },
                TransportFeedbackEntry { sequence: 2, recv_delta_us: 35_000 },
                TransportFeedbackEntry { sequence: 3, recv_delta_us: 55_000 },
            ];
            gcc.process_feedback(&entries);
        }
        assert!(
            gcc.bitrate_multiplier() <= 0.95,
            "Expected multiplier <= 0.95 for congestion, got {}",
            gcc.bitrate_multiplier()
        );
    }

    #[test]
    fn test_process_feedback_empty_and_single() {
        let mut gcc = GccEstimator::new(80_000_000);

        // Empty
        gcc.process_feedback(&[]);
        assert_eq!(gcc.delay_gradient_ms(), 0.0);
        assert_eq!(gcc.trend(), DelayTrend::Normal);

        // Single entry
        gcc.process_feedback(&[TransportFeedbackEntry {
            sequence: 42,
            recv_delta_us: 5_000,
        }]);
        assert_eq!(gcc.delay_gradient_ms(), 0.0);
        assert_eq!(gcc.trend(), DelayTrend::Normal);
    }
}
