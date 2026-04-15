use std::time::{Duration, Instant};
use crate::adaptive::bandwidth_estimator::BandwidthEstimator;
use super::burst_detector::{BurstDetector, LossPattern};
use super::gcc_estimator::GccEstimator;

/// Adaptive bitrate controller.
/// Adjusts encoding bitrate based on network quality estimates.
pub struct BitrateController {
    current_bitrate_bps: u64,
    min_bitrate_bps: u64,
    max_bitrate_bps: u64,
    target_loss_rate: f64,
    last_adjustment: Instant,
    /// Minimum interval between upward adjustments (hysteresis)
    hysteresis_duration: Duration,
}

impl BitrateController {
    pub fn new(initial_bitrate_mbps: u32) -> Self {
        Self {
            current_bitrate_bps: initial_bitrate_mbps as u64 * 1_000_000,
            min_bitrate_bps: 10_000_000,   // 10 Mbps floor
            max_bitrate_bps: 200_000_000,  // 200 Mbps ceiling
            target_loss_rate: 0.02,        // 2%
            last_adjustment: Instant::now(),
            hysteresis_duration: Duration::from_secs(10),
        }
    }

    /// Constructor with custom hysteresis duration (for testing).
    #[cfg(test)]
    pub(crate) fn new_with_hysteresis(initial_bitrate_mbps: u32, hysteresis: Duration) -> Self {
        Self {
            hysteresis_duration: hysteresis,
            ..Self::new(initial_bitrate_mbps)
        }
    }

    /// Evaluate network conditions and adjust bitrate.
    /// Call this periodically (every ~1 second).
    /// Returns true if bitrate was changed.
    pub fn adjust(&mut self, estimator: &BandwidthEstimator, gcc: &GccEstimator, burst: &BurstDetector) -> bool {
        if !estimator.has_data() {
            return false;
        }

        // Burst loss (Wi-Fi interference): skip bitrate changes, let FEC absorb it
        if burst.pattern() == LossPattern::Burst {
            log::info!("Burst loss detected — skipping bitrate adjustment (FEC absorbs)");
            return false;
        }

        let loss = estimator.loss_rate();
        let gradient = gcc.delay_gradient_ms();
        let mut multiplier = 1.0f64;

        // Sustained loss: aggressive reduction regardless of delay signal
        if burst.pattern() == LossPattern::Sustained {
            multiplier = 0.80;
            log::warn!("Sustained loss — aggressive bitrate reduction");
        } else {
            // Delay-based detection: react to congestion before loss occurs
            if gradient > 2.0 {
                multiplier = multiplier.min(0.90);
                log::warn!("Delay overuse (gradient {:.1}ms)", gradient);
            }

            // Loss-based detection: stronger reduction if packets are actually lost
            if loss > 0.05 {
                multiplier = multiplier.min(0.80);
                log::warn!("High packet loss ({:.1}%)", loss * 100.0);
            } else if loss > self.target_loss_rate {
                multiplier = multiplier.min(0.95);
                log::info!("Moderate packet loss ({:.1}%)", loss * 100.0);
            }
        }

        // Increase conditions (only if no reduction)
        if multiplier >= 1.0 {
            if loss < 0.01 && gradient < -1.0 && self.last_adjustment.elapsed() > self.hysteresis_duration {
                multiplier = 1.05;
                log::info!("Low loss + delay recovery (gradient {:.1}ms)", gradient);
            } else if loss < 0.01 && self.last_adjustment.elapsed() > self.hysteresis_duration {
                multiplier = 1.05;
                log::info!("Low packet loss ({:.1}%)", loss * 100.0);
            }
        }

        if (multiplier - 1.0).abs() > f64::EPSILON {
            self.current_bitrate_bps = (self.current_bitrate_bps as f64 * multiplier) as u64;
            self.current_bitrate_bps = self.current_bitrate_bps.max(self.min_bitrate_bps).min(self.max_bitrate_bps);
            self.last_adjustment = Instant::now();
            log::info!("Bitrate adjusted → {} Mbps", self.current_bitrate_bps / 1_000_000);
            return true;
        }

        false
    }

    pub fn current_bitrate_bps(&self) -> u64 { self.current_bitrate_bps }
    pub fn current_bitrate_mbps(&self) -> u32 { (self.current_bitrate_bps / 1_000_000) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_bitrate() {
        let ctrl = BitrateController::new(80);
        assert_eq!(ctrl.current_bitrate_mbps(), 80);
    }

    #[test]
    fn test_high_loss_reduces_bitrate() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(50, 50, 10.0); // 50% loss
        ctrl.adjust(&est, &gcc, &burst);
        assert!(ctrl.current_bitrate_mbps() < 100);
    }

    #[test]
    fn test_no_loss_no_immediate_increase() {
        let mut ctrl = BitrateController::new(80);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss
        let changed = ctrl.adjust(&est, &gcc, &burst);
        // Should not increase yet (hysteresis)
        assert!(!changed);
        assert_eq!(ctrl.current_bitrate_mbps(), 80);
    }

    #[test]
    fn test_floor_enforced() {
        let mut ctrl = BitrateController::new(11);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(10, 90, 100.0); // 90% loss - extreme
        ctrl.adjust(&est, &gcc, &burst); // → 11 * 0.8 = 8.8 → clamped to 10
        assert_eq!(ctrl.current_bitrate_mbps(), 10);
    }

    #[test]
    fn test_moderate_loss_gentle_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(97, 3, 8.0); // 3% loss
        ctrl.adjust(&est, &gcc, &burst);
        assert_eq!(ctrl.current_bitrate_mbps(), 95); // -5%
    }

    #[test]
    fn test_adjust_overuse_without_loss() {
        use fvp_common::protocol::TransportFeedbackEntry;
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let mut gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss

        // Simulate congestion: increasing inter-arrival deltas
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 10_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 13_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 17_000 },
            TransportFeedbackEntry { sequence: 3, recv_delta_us: 22_000 },
        ];
        gcc.process_feedback(&entries);
        assert!(gcc.delay_gradient_ms() > 2.0, "gradient should be >2.0, got {}", gcc.delay_gradient_ms());

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(changed, "Bitrate should have decreased");
        assert!(ctrl.current_bitrate_mbps() < 100, "Expected reduction, got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_adjust_delay_and_loss_combined() {
        use fvp_common::protocol::TransportFeedbackEntry;
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let mut gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(90, 10, 10.0); // 10% loss (high)

        // Also simulate delay overuse
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 10_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 15_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 22_000 },
        ];
        gcc.process_feedback(&entries);

        ctrl.adjust(&est, &gcc, &burst);
        // Max-of-reductions: loss -20% dominates delay -10%, so 100 * 0.80 = 80 Mbps
        assert_eq!(ctrl.current_bitrate_mbps(), 80, "Expected max reduction (0.80), got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_underuse_increases_bitrate() {
        use fvp_common::protocol::TransportFeedbackEntry;
        // Use short hysteresis so we don't need to sleep 10 seconds
        let mut ctrl = BitrateController::new_with_hysteresis(100, Duration::from_millis(10));
        let mut est = BandwidthEstimator::new();
        let mut gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss

        // Simulate delay recovery: decreasing inter-arrival deltas (gradient < -1.0)
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 20_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 17_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 13_000 },
            TransportFeedbackEntry { sequence: 3, recv_delta_us: 8_000 },
        ];
        gcc.process_feedback(&entries);
        assert!(gcc.delay_gradient_ms() < -1.0, "gradient should be < -1.0, got {}", gcc.delay_gradient_ms());

        // Wait for hysteresis to elapse
        std::thread::sleep(Duration::from_millis(20));

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(changed, "Bitrate should have increased");
        assert_eq!(ctrl.current_bitrate_mbps(), 105, "Expected +5% increase, got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_ceiling_enforced() {
        let mut ctrl = BitrateController::new_with_hysteresis(195, Duration::from_millis(10));
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss, no congestion

        // Wait for hysteresis to elapse
        std::thread::sleep(Duration::from_millis(20));

        ctrl.adjust(&est, &gcc, &burst);
        // 195 * 1.05 = 204.75 → clamped to 200 Mbps ceiling
        assert_eq!(ctrl.current_bitrate_mbps(), 200, "Expected ceiling at 200 Mbps, got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_no_change_without_data() {
        let mut ctrl = BitrateController::new(100);
        let est = BandwidthEstimator::new(); // no data fed
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(!changed, "Should not change without data");
        assert_eq!(ctrl.current_bitrate_mbps(), 100);
    }

    #[test]
    fn test_burst_suppresses_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let mut burst = BurstDetector::new();
        est.update(50, 50, 10.0); // 50% loss — very high
        burst.record(0.50); // Triggers Burst pattern
        assert_eq!(burst.pattern(), LossPattern::Burst);

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(!changed, "Burst should suppress bitrate reduction");
        assert_eq!(ctrl.current_bitrate_mbps(), 100, "Bitrate should remain unchanged during burst");
    }

    #[test]
    fn test_sustained_triggers_aggressive_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let mut burst = BurstDetector::new();
        est.update(90, 10, 10.0); // 10% loss
        burst.record(0.10); // Start burst
        // Wait beyond sustained threshold
        std::thread::sleep(Duration::from_millis(600));
        burst.record(0.10); // Now sustained
        assert_eq!(burst.pattern(), LossPattern::Sustained);

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(changed, "Sustained loss should trigger bitrate change");
        assert_eq!(ctrl.current_bitrate_mbps(), 80, "Expected aggressive -20% reduction, got {}", ctrl.current_bitrate_mbps());
    }
}
