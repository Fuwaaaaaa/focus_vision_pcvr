use std::time::Instant;

/// Estimates network quality using EWMA of packet loss rate and RTT.
pub struct BandwidthEstimator {
    /// Exponentially weighted moving average of packet loss rate (0.0 - 1.0)
    loss_rate_ewma: f64,
    /// EWMA of round-trip time in milliseconds
    rtt_ms_ewma: f64,
    /// EWMA smoothing factor (0..1, higher = more recent weight)
    alpha: f64,
    /// Last update time
    last_update: Instant,
    /// Whether we have received at least one report
    has_data: bool,
}

impl Default for BandwidthEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl BandwidthEstimator {
    pub fn new() -> Self {
        Self {
            loss_rate_ewma: 0.0,
            rtt_ms_ewma: 10.0, // assume 10ms initially
            alpha: 0.3,
            last_update: Instant::now(),
            has_data: false,
        }
    }

    /// Update with a stats report from the HMD.
    pub fn update(&mut self, packets_received: u32, packets_lost: u32, rtt_ms: f64) {
        let total = packets_received + packets_lost;
        let loss = if total > 0 {
            packets_lost as f64 / total as f64
        } else {
            0.0
        };

        if self.has_data {
            self.loss_rate_ewma = self.alpha * loss + (1.0 - self.alpha) * self.loss_rate_ewma;
            self.rtt_ms_ewma = self.alpha * rtt_ms + (1.0 - self.alpha) * self.rtt_ms_ewma;
        } else {
            self.loss_rate_ewma = loss;
            self.rtt_ms_ewma = rtt_ms;
            self.has_data = true;
        }

        self.last_update = Instant::now();
    }

    pub fn loss_rate(&self) -> f64 { self.loss_rate_ewma }
    pub fn rtt_ms(&self) -> f64 { self.rtt_ms_ewma }
    pub fn has_data(&self) -> bool { self.has_data }
    pub fn last_update(&self) -> Instant { self.last_update }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let est = BandwidthEstimator::new();
        assert!(!est.has_data());
        assert_eq!(est.loss_rate(), 0.0);
    }

    #[test]
    fn test_update_no_loss() {
        let mut est = BandwidthEstimator::new();
        est.update(100, 0, 5.0);
        assert!(est.has_data());
        assert_eq!(est.loss_rate(), 0.0);
        assert_eq!(est.rtt_ms(), 5.0);
    }

    #[test]
    fn test_update_with_loss() {
        let mut est = BandwidthEstimator::new();
        est.update(90, 10, 8.0); // 10% loss
        assert!((est.loss_rate() - 0.1).abs() < 0.01);
    }

    #[test]
    fn test_ewma_smoothing() {
        let mut est = BandwidthEstimator::new();
        est.update(100, 0, 5.0); // 0% loss
        est.update(50, 50, 5.0); // 50% loss
        // EWMA should be between 0 and 0.5
        assert!(est.loss_rate() > 0.0);
        assert!(est.loss_rate() < 0.5);
    }

    #[test]
    fn test_zero_packets_both_zero() {
        let mut est = BandwidthEstimator::new();
        est.update(0, 0, 5.0);
        assert_eq!(est.loss_rate(), 0.0);
        assert!(est.has_data());
    }

    #[test]
    fn test_all_packets_lost() {
        let mut est = BandwidthEstimator::new();
        est.update(0, 100, 10.0); // 100% loss
        assert!((est.loss_rate() - 1.0).abs() < 0.01);
    }
}
