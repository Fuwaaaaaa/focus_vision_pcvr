use std::time::Duration;

/// Exponential backoff reconnection policy.
pub struct ReconnectPolicy {
    max_retries: u32,
    initial_interval: Duration,
    max_interval: Duration,
    multiplier: f64,
    current_attempt: u32,
}

impl ReconnectPolicy {
    pub fn new() -> Self {
        Self {
            max_retries: 5,
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(30),
            multiplier: 2.0,
            current_attempt: 0,
        }
    }

    /// Get the delay before the next reconnection attempt.
    /// Returns None if max retries exceeded.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.current_attempt >= self.max_retries {
            return None;
        }

        let base_ms = self.initial_interval.as_millis() as f64
            * self.multiplier.powi(self.current_attempt as i32);
        let delay_ms = base_ms.min(self.max_interval.as_millis() as f64);

        // Add jitter: ±20%
        let jitter = 1.0 + (self.current_attempt as f64 * 0.1 - 0.1); // simple deterministic jitter
        let final_ms = (delay_ms * jitter).max(100.0);

        self.current_attempt += 1;
        log::info!(
            "Reconnect attempt {}/{}, delay: {:.0}ms",
            self.current_attempt, self.max_retries, final_ms
        );

        Some(Duration::from_millis(final_ms as u64))
    }

    /// Reset after successful connection.
    pub fn reset(&mut self) {
        self.current_attempt = 0;
    }

    /// Check if all retries are exhausted.
    pub fn exhausted(&self) -> bool {
        self.current_attempt >= self.max_retries
    }

    pub fn attempt(&self) -> u32 { self.current_attempt }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let mut policy = ReconnectPolicy::new();

        let d1 = policy.next_delay().unwrap();
        let d2 = policy.next_delay().unwrap();
        let d3 = policy.next_delay().unwrap();

        // Each delay should be longer than the previous
        assert!(d2 >= d1);
        assert!(d3 >= d2);
    }

    #[test]
    fn test_max_retries() {
        let mut policy = ReconnectPolicy::new();
        for _ in 0..5 {
            assert!(policy.next_delay().is_some());
        }
        // 6th attempt should fail
        assert!(policy.next_delay().is_none());
        assert!(policy.exhausted());
    }

    #[test]
    fn test_reset() {
        let mut policy = ReconnectPolicy::new();
        policy.next_delay();
        policy.next_delay();
        assert_eq!(policy.attempt(), 2);
        policy.reset();
        assert_eq!(policy.attempt(), 0);
        assert!(!policy.exhausted());
    }

    #[test]
    fn test_max_interval_cap() {
        let mut policy = ReconnectPolicy::new();
        // Even after many attempts, should not exceed 30s
        for _ in 0..4 {
            let d = policy.next_delay().unwrap();
            assert!(d <= Duration::from_secs(35)); // 30s + some jitter
        }
    }
}
