//! Codec benchmark: compares H.265 vs H.264 decode latency from HMD reports.
//!
//! Flow:
//! 1. Start with H.265 for 5 seconds, collect decode latency samples
//! 2. Switch to H.264 for 5 seconds, collect decode latency samples
//! 3. Compare averages, select the codec with lower latency
//! 4. Save result to config/local.toml

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BenchPhase {
    NotStarted,
    TestingHevc,
    TestingH264,
    Complete,
}

#[derive(Debug, Clone, Copy)]
pub enum CodecChoice {
    Hevc,
    H264,
}

pub struct CodecBenchmark {
    phase: BenchPhase,
    phase_start: Option<Instant>,
    phase_duration: Duration,

    hevc_samples: Vec<u32>,
    h264_samples: Vec<u32>,

    result: Option<CodecChoice>,
}

impl Default for CodecBenchmark {
    fn default() -> Self {
        Self::new()
    }
}

impl CodecBenchmark {
    pub fn new() -> Self {
        Self {
            phase: BenchPhase::NotStarted,
            phase_start: None,
            phase_duration: Duration::from_secs(5),
            hevc_samples: Vec::with_capacity(450),
            h264_samples: Vec::with_capacity(450),
            result: None,
        }
    }

    /// Start the benchmark sequence.
    pub fn start(&mut self) {
        self.phase = BenchPhase::TestingHevc;
        self.phase_start = Some(Instant::now());
        self.hevc_samples.clear();
        self.h264_samples.clear();
        self.result = None;
        log::info!("Codec benchmark started: testing H.265");
    }

    /// Record a decode latency sample from the HMD.
    pub fn record_sample(&mut self, decode_latency_us: u32) {
        match self.phase {
            BenchPhase::TestingHevc => self.hevc_samples.push(decode_latency_us),
            BenchPhase::TestingH264 => self.h264_samples.push(decode_latency_us),
            _ => {}
        }
    }

    /// Tick the benchmark state machine. Returns the codec to use RIGHT NOW.
    /// Call this every frame or heartbeat interval.
    pub fn tick(&mut self) -> Option<CodecChoice> {
        match self.phase {
            BenchPhase::NotStarted => None,
            BenchPhase::TestingHevc => {
                if self.phase_start.is_some_and(|s| s.elapsed() >= self.phase_duration) {
                    self.phase = BenchPhase::TestingH264;
                    self.phase_start = Some(Instant::now());
                    log::info!(
                        "Codec benchmark: H.265 done ({} samples, avg {}us). Switching to H.264.",
                        self.hevc_samples.len(),
                        Self::avg(&self.hevc_samples)
                    );
                    return Some(CodecChoice::H264);
                }
                Some(CodecChoice::Hevc)
            }
            BenchPhase::TestingH264 => {
                if self.phase_start.is_some_and(|s| s.elapsed() >= self.phase_duration) {
                    self.finalize();
                    return self.result;
                }
                Some(CodecChoice::H264)
            }
            BenchPhase::Complete => self.result,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.phase == BenchPhase::Complete
    }

    pub fn result(&self) -> Option<CodecChoice> {
        self.result
    }

    fn finalize(&mut self) {
        let hevc_avg = Self::avg(&self.hevc_samples);
        let h264_avg = Self::avg(&self.h264_samples);

        self.result = if h264_avg < hevc_avg && !self.h264_samples.is_empty() {
            Some(CodecChoice::H264)
        } else {
            Some(CodecChoice::Hevc)
        };

        self.phase = BenchPhase::Complete;
        log::info!(
            "Codec benchmark complete: H.265={}us ({} samples), H.264={}us ({} samples) → {:?}",
            hevc_avg, self.hevc_samples.len(),
            h264_avg, self.h264_samples.len(),
            self.result.unwrap()
        );
    }

    fn avg(samples: &[u32]) -> u32 {
        if samples.is_empty() {
            return 0;
        }
        (samples.iter().map(|&s| s as u64).sum::<u64>() / samples.len() as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_selects_faster_codec() {
        let mut bench = CodecBenchmark::new();
        bench.phase_duration = Duration::from_millis(10);
        bench.start();

        // Simulate H.265 samples (slower: 5000us avg)
        for _ in 0..10 {
            bench.record_sample(5000);
        }

        // Wait for phase transition
        std::thread::sleep(Duration::from_millis(15));
        bench.tick();

        // Simulate H.264 samples (faster: 3000us avg)
        for _ in 0..10 {
            bench.record_sample(3000);
        }

        std::thread::sleep(Duration::from_millis(15));
        bench.tick();

        assert!(bench.is_complete());
        assert!(matches!(bench.result(), Some(CodecChoice::H264)));
    }

    #[test]
    fn test_benchmark_defaults_to_hevc_on_tie() {
        let mut bench = CodecBenchmark::new();
        bench.phase_duration = Duration::from_millis(10);
        bench.start();

        for _ in 0..10 {
            bench.record_sample(4000);
        }

        std::thread::sleep(Duration::from_millis(15));
        bench.tick();

        for _ in 0..10 {
            bench.record_sample(4000);
        }

        std::thread::sleep(Duration::from_millis(15));
        bench.tick();

        assert!(bench.is_complete());
        assert!(matches!(bench.result(), Some(CodecChoice::Hevc)));
    }
}
