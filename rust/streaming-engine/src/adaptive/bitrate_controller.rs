use std::time::{Duration, Instant};
use crate::adaptive::bandwidth_estimator::BandwidthEstimator;

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

    /// Evaluate network conditions and adjust bitrate.
    /// Call this periodically (every ~1 second).
    /// Returns true if bitrate was changed.
    pub fn adjust(&mut self, estimator: &BandwidthEstimator) -> bool {
        if !estimator.has_data() {
            return false;
        }

        let loss = estimator.loss_rate();
        let old_bitrate = self.current_bitrate_bps;

        if loss > 0.05 {
            // High loss (>5%): aggressive reduction -20%
            self.current_bitrate_bps = (self.current_bitrate_bps as f64 * 0.80) as u64;
            self.last_adjustment = Instant::now();
            log::warn!("High packet loss ({:.1}%), bitrate -20% → {} Mbps",
                loss * 100.0, self.current_bitrate_bps / 1_000_000);
        } else if loss > self.target_loss_rate {
            // Moderate loss (>2%): gentle reduction -5%
            self.current_bitrate_bps = (self.current_bitrate_bps as f64 * 0.95) as u64;
            self.last_adjustment = Instant::now();
            log::info!("Moderate packet loss ({:.1}%), bitrate -5% → {} Mbps",
                loss * 100.0, self.current_bitrate_bps / 1_000_000);
        } else if loss < 0.01
            && self.last_adjustment.elapsed() > self.hysteresis_duration
        {
            // Low loss (<1%) and stable for 10s: cautious increase +5%
            self.current_bitrate_bps = (self.current_bitrate_bps as f64 * 1.05) as u64;
            self.last_adjustment = Instant::now();
            log::info!("Low packet loss ({:.1}%), bitrate +5% → {} Mbps",
                loss * 100.0, self.current_bitrate_bps / 1_000_000);
        }

        // Clamp
        self.current_bitrate_bps = self.current_bitrate_bps
            .max(self.min_bitrate_bps)
            .min(self.max_bitrate_bps);

        self.current_bitrate_bps != old_bitrate
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
        est.update(50, 50, 10.0); // 50% loss
        ctrl.adjust(&est);
        assert!(ctrl.current_bitrate_mbps() < 100);
    }

    #[test]
    fn test_no_loss_no_immediate_increase() {
        let mut ctrl = BitrateController::new(80);
        let mut est = BandwidthEstimator::new();
        est.update(100, 0, 5.0); // 0% loss
        let changed = ctrl.adjust(&est);
        // Should not increase yet (hysteresis)
        assert!(!changed);
        assert_eq!(ctrl.current_bitrate_mbps(), 80);
    }

    #[test]
    fn test_floor_enforced() {
        let mut ctrl = BitrateController::new(11);
        let mut est = BandwidthEstimator::new();
        est.update(10, 90, 100.0); // 90% loss - extreme
        ctrl.adjust(&est); // → 11 * 0.8 = 8.8 → clamped to 10
        assert_eq!(ctrl.current_bitrate_mbps(), 10);
    }

    #[test]
    fn test_moderate_loss_gentle_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        est.update(97, 3, 8.0); // 3% loss
        ctrl.adjust(&est);
        assert_eq!(ctrl.current_bitrate_mbps(), 95); // -5%
    }
}
