use std::collections::VecDeque;

/// Ring buffer for 30 seconds of streaming stats (sampled at 1Hz).
pub struct StatsHistory {
    pub latency_ms: VecDeque<f32>,
    pub fps: VecDeque<f32>,
    pub packet_loss: VecDeque<f32>,
    max_points: usize,
}

impl StatsHistory {
    pub fn new() -> Self {
        Self {
            latency_ms: VecDeque::with_capacity(30),
            fps: VecDeque::with_capacity(30),
            packet_loss: VecDeque::with_capacity(30),
            max_points: 30,
        }
    }

    pub fn push(&mut self, latency_ms: f32, fps: f32, packet_loss: f32) {
        Self::push_ring(&mut self.latency_ms, latency_ms, self.max_points);
        Self::push_ring(&mut self.fps, fps, self.max_points);
        Self::push_ring(&mut self.packet_loss, packet_loss, self.max_points);
    }

    /// Convert to egui_plot-compatible points: [(x=index, y=value)].
    pub fn as_plot_points(series: &VecDeque<f32>) -> Vec<[f64; 2]> {
        series
            .iter()
            .enumerate()
            .map(|(i, &v)| [i as f64, v as f64])
            .collect()
    }

    fn push_ring(buf: &mut VecDeque<f32>, value: f32, max: usize) {
        if buf.len() >= max {
            buf.pop_front();
        }
        buf.push_back(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_respects_max_points_ring_buffer_limit() {
        let mut h = StatsHistory::new();
        for i in 0..50 {
            h.push(i as f32, i as f32, i as f32);
        }
        assert_eq!(h.latency_ms.len(), 30);
        assert_eq!(h.fps.len(), 30);
        assert_eq!(h.packet_loss.len(), 30);
    }

    #[test]
    fn empty_buffer_produces_empty_plot_points() {
        let h = StatsHistory::new();
        let points = StatsHistory::as_plot_points(&h.latency_ms);
        assert!(points.is_empty());
    }

    #[test]
    fn full_buffer_overflow_drops_oldest() {
        let mut h = StatsHistory::new();
        for i in 0..35 {
            h.push(i as f32, 0.0, 0.0);
        }
        // Oldest should be 5.0 (items 0..4 were dropped)
        assert_eq!(h.latency_ms.len(), 30);
        assert_eq!(*h.latency_ms.front().unwrap(), 5.0);
        assert_eq!(*h.latency_ms.back().unwrap(), 34.0);
    }

    #[test]
    fn as_plot_points_conversion_accuracy() {
        let mut h = StatsHistory::new();
        h.push(10.0, 60.0, 0.5);
        h.push(20.0, 90.0, 1.0);
        h.push(15.0, 75.0, 0.0);

        let points = StatsHistory::as_plot_points(&h.latency_ms);
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], [0.0, 10.0]);
        assert_eq!(points[1], [1.0, 20.0]);
        assert_eq!(points[2], [2.0, 15.0]);
    }

    #[test]
    fn push_exactly_max_points_items() {
        let mut h = StatsHistory::new();
        for i in 0..30 {
            h.push(i as f32, i as f32, i as f32);
        }
        assert_eq!(h.latency_ms.len(), 30);
        assert_eq!(*h.latency_ms.front().unwrap(), 0.0);
        assert_eq!(*h.latency_ms.back().unwrap(), 29.0);
    }
}
