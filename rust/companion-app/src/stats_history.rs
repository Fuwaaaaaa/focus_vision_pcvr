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
