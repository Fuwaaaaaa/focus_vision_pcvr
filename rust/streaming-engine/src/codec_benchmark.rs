//! Codec benchmark: compares H.265 vs H.264 decode latency from HMD reports.
//!
//! Flow:
//! 1. Start with H.265 for 5 seconds, collect decode latency samples
//! 2. Switch to H.264 for 5 seconds, collect decode latency samples
//! 3. Compare averages, select the codec with lower latency
//! 4. Save result to config/local.toml

use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

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

    /// Export benchmark results as a structured report.
    pub fn export_results(&self) -> Option<BenchmarkReport> {
        if self.phase != BenchPhase::Complete {
            return None;
        }
        Some(BenchmarkReport {
            hevc: BenchmarkResult::from_samples("h265", &self.hevc_samples),
            h264: BenchmarkResult::from_samples("h264", &self.h264_samples),
            selected: match self.result? {
                CodecChoice::Hevc => "h265".to_string(),
                CodecChoice::H264 => "h264".to_string(),
            },
            timestamp: chrono_timestamp(),
        })
    }

    fn avg(samples: &[u32]) -> u32 {
        if samples.is_empty() {
            return 0;
        }
        (samples.iter().map(|&s| s as u64).sum::<u64>() / samples.len() as u64) as u32
    }

    fn stddev(samples: &[u32]) -> f64 {
        if samples.len() < 2 {
            return 0.0;
        }
        let mean = samples.iter().map(|&s| s as f64).sum::<f64>() / samples.len() as f64;
        let variance = samples.iter()
            .map(|&s| { let d = s as f64 - mean; d * d })
            .sum::<f64>() / (samples.len() - 1) as f64;
        variance.sqrt()
    }
}

/// Structured benchmark result for one codec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub codec: String,
    pub avg_decode_us: u32,
    pub sample_count: usize,
    pub stddev_us: f64,
    pub min_us: u32,
    pub max_us: u32,
}

impl BenchmarkResult {
    fn from_samples(codec: &str, samples: &[u32]) -> Self {
        Self {
            codec: codec.to_string(),
            avg_decode_us: CodecBenchmark::avg(samples),
            sample_count: samples.len(),
            stddev_us: CodecBenchmark::stddev(samples),
            min_us: samples.iter().copied().min().unwrap_or(0),
            max_us: samples.iter().copied().max().unwrap_or(0),
        }
    }
}

/// Complete benchmark report with both codecs and selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub hevc: BenchmarkResult,
    pub h264: BenchmarkResult,
    pub selected: String,
    pub timestamp: String,
}

impl BenchmarkReport {
    /// Save report to JSON file.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Load report from JSON file.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(std::io::Error::other)
    }
}

fn chrono_timestamp() -> String {
    // Simple ISO-8601 timestamp without external dependency
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{}", now)
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
    fn test_benchmark_result_from_samples() {
        let samples = vec![1000, 2000, 3000, 4000, 5000];
        let result = BenchmarkResult::from_samples("h265", &samples);
        assert_eq!(result.codec, "h265");
        assert_eq!(result.avg_decode_us, 3000);
        assert_eq!(result.sample_count, 5);
        assert_eq!(result.min_us, 1000);
        assert_eq!(result.max_us, 5000);
        assert!(result.stddev_us > 0.0);
    }

    #[test]
    fn test_benchmark_result_empty_samples() {
        let result = BenchmarkResult::from_samples("h264", &[]);
        assert_eq!(result.avg_decode_us, 0);
        assert_eq!(result.sample_count, 0);
        assert_eq!(result.min_us, 0);
        assert_eq!(result.max_us, 0);
        assert_eq!(result.stddev_us, 0.0);
    }

    #[test]
    fn test_benchmark_report_json_roundtrip() {
        let report = BenchmarkReport {
            hevc: BenchmarkResult::from_samples("h265", &[3000, 3100, 2900]),
            h264: BenchmarkResult::from_samples("h264", &[2500, 2600, 2400]),
            selected: "h264".to_string(),
            timestamp: "1234567890".to_string(),
        };

        let json = serde_json::to_string(&report).unwrap();
        let loaded: BenchmarkReport = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.selected, "h264");
        assert_eq!(loaded.hevc.avg_decode_us, 3000);
        assert_eq!(loaded.h264.avg_decode_us, 2500);
        assert_eq!(loaded.hevc.sample_count, 3);
    }

    #[test]
    fn test_benchmark_report_save_load() {
        let report = BenchmarkReport {
            hevc: BenchmarkResult::from_samples("h265", &[4000, 4100]),
            h264: BenchmarkResult::from_samples("h264", &[3500, 3600]),
            selected: "h264".to_string(),
            timestamp: "9999".to_string(),
        };

        let dir = std::env::temp_dir().join("fvp_test_benchmark");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("benchmark_results.json");

        report.save(&path).unwrap();
        let loaded = BenchmarkReport::load(&path).unwrap();

        assert_eq!(loaded.selected, "h264");
        assert_eq!(loaded.hevc.sample_count, 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_export_results_before_complete_returns_none() {
        let bench = CodecBenchmark::new();
        assert!(bench.export_results().is_none());
    }

    #[test]
    fn test_stddev_calculation() {
        // stddev of [2, 4, 4, 4, 5, 5, 7, 9] ≈ 2.14
        let samples = vec![2, 4, 4, 4, 5, 5, 7, 9];
        let sd = CodecBenchmark::stddev(&samples);
        assert!((sd - 2.14).abs() < 0.1);
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
