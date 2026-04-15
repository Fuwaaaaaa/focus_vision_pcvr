//! Distinguishes transient burst loss (Wi-Fi interference) from sustained congestion.
//!
//! Burst: >=5% loss within 100ms window, preceded and followed by no loss.
//! Sustained: loss continues for >500ms.

use std::time::{Duration, Instant};

/// Classification of the current loss pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossPattern {
    /// No significant loss detected
    None,
    /// Transient burst — recommend temporary FEC increase, not bitrate reduction
    Burst,
    /// Sustained loss — recommend bitrate reduction
    Sustained,
}

/// Tracks packet loss events to classify burst vs sustained patterns.
pub struct BurstDetector {
    pattern: LossPattern,
    burst_start: Option<Instant>,
    sustained_threshold: Duration,
    loss_rate_threshold: f64,
}

impl BurstDetector {
    pub fn new() -> Self {
        Self {
            pattern: LossPattern::None,
            burst_start: None,
            sustained_threshold: Duration::from_millis(500),
            loss_rate_threshold: 0.05,
        }
    }

    /// Record a loss measurement. Call periodically (e.g., every heartbeat).
    /// `loss_rate` is 0.0-1.0 (fraction of packets lost in this interval).
    pub fn record(&mut self, loss_rate: f64) {
        if loss_rate >= self.loss_rate_threshold {
            match self.burst_start {
                None => {
                    self.burst_start = Some(Instant::now());
                    self.pattern = LossPattern::Burst;
                }
                Some(start) => {
                    if start.elapsed() > self.sustained_threshold {
                        self.pattern = LossPattern::Sustained;
                    }
                }
            }
        } else {
            if self.burst_start.is_some() {
                self.burst_start = None;
                self.pattern = LossPattern::None;
            }
        }
    }

    pub fn pattern(&self) -> LossPattern { self.pattern }
    pub fn recommend_fec_boost(&self) -> bool { self.pattern == LossPattern::Burst }
    pub fn recommend_bitrate_reduction(&self) -> bool { self.pattern == LossPattern::Sustained }
}

impl Default for BurstDetector {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_initial_state() {
        let bd = BurstDetector::new();
        assert_eq!(bd.pattern(), LossPattern::None);
        assert!(!bd.recommend_fec_boost());
        assert!(!bd.recommend_bitrate_reduction());
    }

    #[test]
    fn test_no_loss_stays_none() {
        let mut bd = BurstDetector::new();
        bd.record(0.0);
        bd.record(0.01);
        bd.record(0.0);
        assert_eq!(bd.pattern(), LossPattern::None);
    }

    #[test]
    fn test_brief_loss_is_burst() {
        let mut bd = BurstDetector::new();
        bd.record(0.10); // 10% loss — above threshold
        assert_eq!(bd.pattern(), LossPattern::Burst);
        assert!(bd.recommend_fec_boost());
        assert!(!bd.recommend_bitrate_reduction());
    }

    #[test]
    fn test_sustained_loss_detected() {
        let mut bd = BurstDetector::new();
        bd.record(0.10); // Start burst
        assert_eq!(bd.pattern(), LossPattern::Burst);

        // Wait beyond sustained threshold
        thread::sleep(Duration::from_millis(600));

        bd.record(0.10); // Still losing
        assert_eq!(bd.pattern(), LossPattern::Sustained);
        assert!(!bd.recommend_fec_boost());
        assert!(bd.recommend_bitrate_reduction());
    }

    #[test]
    fn test_below_threshold_ignored() {
        let mut bd = BurstDetector::new();
        bd.record(0.04); // Below 5% threshold
        assert_eq!(bd.pattern(), LossPattern::None);
        bd.record(0.049);
        assert_eq!(bd.pattern(), LossPattern::None);
    }

    #[test]
    fn test_burst_then_recovery() {
        let mut bd = BurstDetector::new();
        bd.record(0.10); // Burst starts
        assert_eq!(bd.pattern(), LossPattern::Burst);

        bd.record(0.01); // Loss stops — recovery
        assert_eq!(bd.pattern(), LossPattern::None);
        assert!(!bd.recommend_fec_boost());
        assert!(!bd.recommend_bitrate_reduction());
    }
}
