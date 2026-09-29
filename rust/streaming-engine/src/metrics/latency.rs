use std::time::{Duration, Instant};
use std::collections::VecDeque;

/// Per-frame timestamps for latency profiling.
#[derive(Debug, Clone)]
pub struct FrameTimestamps {
    pub frame_index: u32,
    /// When the frame was ready to encode (or, when the encode time is
    /// unknown, when it reached the engine).
    pub t_present: Instant,
    pub t_encode_start: Option<Instant>,
    pub t_encode_end: Option<Instant>,
    pub t_send: Option<Instant>,
}

impl FrameTimestamps {
    /// A frame reaching the engine now, its encode time unknown (the
    /// simulator's synthetic frames).
    pub fn new(frame_index: u32) -> Self {
        Self {
            frame_index,
            t_present: Instant::now(),
            t_encode_start: None,
            t_encode_end: None,
            t_send: None,
        }
    }

    /// A frame reaching the engine now that the driver took `encode` to
    /// compose and encode: it was ready that long ago.
    pub fn encoded(frame_index: u32, encode: Duration) -> Self {
        let now = Instant::now();
        let start = now.checked_sub(encode).unwrap_or(now);
        Self {
            frame_index,
            t_present: start,
            t_encode_start: Some(start),
            t_encode_end: Some(now),
            t_send: None,
        }
    }

    pub fn mark_encode_start(&mut self) {
        self.t_encode_start = Some(Instant::now());
    }

    pub fn mark_encode_end(&mut self) {
        self.t_encode_end = Some(Instant::now());
    }

    pub fn mark_send(&mut self) {
        self.t_send = Some(Instant::now());
    }

    /// Total PC-side latency: present → send
    pub fn pc_latency_us(&self) -> Option<u64> {
        self.t_send.map(|t| t.duration_since(self.t_present).as_micros() as u64)
    }

    /// Encode latency only
    pub fn encode_latency_us(&self) -> Option<u64> {
        match (self.t_encode_start, self.t_encode_end) {
            (Some(s), Some(e)) => Some(e.duration_since(s).as_micros() as u64),
            _ => None,
        }
    }
}

/// Rolling latency statistics tracker.
pub struct LatencyTracker {
    history: VecDeque<FrameTimestamps>,
    max_history: usize,
}

impl LatencyTracker {
    pub fn new(max_history: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(max_history),
            max_history,
        }
    }

    pub fn record(&mut self, ts: FrameTimestamps) {
        if self.history.len() >= self.max_history {
            self.history.pop_front();
        }
        self.history.push_back(ts);
    }

    /// Average PC-side latency over recent frames (microseconds).
    pub fn avg_pc_latency_us(&self) -> Option<u64> {
        let (sum, count) = self.history.iter()
            .filter_map(|ts| ts.pc_latency_us())
            .fold((0u64, 0u64), |(s, c), v| (s + v, c + 1));
        if count == 0 { return None; }
        Some(sum / count)
    }

    /// Average encode latency over recent frames (microseconds).
    pub fn avg_encode_latency_us(&self) -> Option<u64> {
        let (sum, count) = self.history.iter()
            .filter_map(|ts| ts.encode_latency_us())
            .fold((0u64, 0u64), |(s, c), v| (s + v, c + 1));
        if count == 0 { return None; }
        Some(sum / count)
    }

    pub fn frame_count(&self) -> usize {
        self.history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_frame_timestamps() {
        let mut ts = FrameTimestamps::new(0);
        ts.mark_encode_start();
        thread::sleep(Duration::from_millis(1));
        ts.mark_encode_end();
        ts.mark_send();

        assert!(ts.encode_latency_us().unwrap() >= 1000); // >= 1ms
        assert!(ts.pc_latency_us().unwrap() >= 1000);
    }

    #[test]
    fn test_latency_tracker() {
        let mut tracker = LatencyTracker::new(10);
        for i in 0..5 {
            let mut ts = FrameTimestamps::new(i);
            ts.mark_encode_start();
            ts.mark_encode_end();
            ts.mark_send();
            tracker.record(ts);
        }
        assert_eq!(tracker.frame_count(), 5);
        assert!(tracker.avg_pc_latency_us().is_some());
    }

    #[test]
    fn test_latency_tracker_rolling_window() {
        let mut tracker = LatencyTracker::new(5);
        for i in 0..10 {
            let mut ts = FrameTimestamps::new(i);
            ts.mark_encode_start();
            ts.mark_encode_end();
            ts.mark_send();
            tracker.record(ts);
        }
        // Should evict oldest, keeping only 5
        assert_eq!(tracker.frame_count(), 5);
    }

    #[test]
    fn test_latency_tracker_empty_returns_none() {
        let tracker = LatencyTracker::new(10);
        assert_eq!(tracker.frame_count(), 0);
        assert!(tracker.avg_pc_latency_us().is_none());
        assert!(tracker.avg_encode_latency_us().is_none());
    }

    #[test]
    fn test_encode_latency_without_start_returns_none() {
        let mut ts = FrameTimestamps::new(0);
        // Only mark encode_end and send, skip encode_start
        ts.mark_encode_end();
        ts.mark_send();
        assert!(ts.encode_latency_us().is_none());
        assert!(ts.pc_latency_us().is_some()); // present→send still works
    }

    #[test]
    fn test_encoded_frame_counts_the_driver_encode() {
        let mut ts = FrameTimestamps::encoded(7, Duration::from_millis(6));
        ts.mark_send();
        let enc = ts.encode_latency_us().unwrap();
        assert!((5_900..=6_100).contains(&enc), "encode {enc} µs");
        assert!(ts.pc_latency_us().unwrap() >= enc, "the PC latency includes the encode");

        let mut tracker = LatencyTracker::new(10);
        tracker.record(ts);
        let mut synthetic = FrameTimestamps::new(8);
        synthetic.mark_send();
        tracker.record(synthetic);
        let avg = tracker.avg_encode_latency_us().unwrap();
        assert!((5_900..=6_100).contains(&avg), "a frame without an encode time doesn't count as 0: {avg}");
    }

    #[test]
    fn test_frame_timestamps_roundtrip() {
        let mut ts = FrameTimestamps::new(42);
        assert_eq!(ts.frame_index, 42);
        assert!(ts.pc_latency_us().is_none()); // no send yet
        assert!(ts.encode_latency_us().is_none()); // no encode yet

        ts.mark_encode_start();
        thread::sleep(Duration::from_millis(1));
        ts.mark_encode_end();
        ts.mark_send();

        let enc = ts.encode_latency_us().unwrap();
        let total = ts.pc_latency_us().unwrap();
        assert!(enc >= 1000); // >= 1ms encode
        assert!(total >= enc); // total >= encode
    }
}
