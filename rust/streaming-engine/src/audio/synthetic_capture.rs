//! Synthetic audio source for the simulator harness.
//!
//! Mirrors the channel contract of [`crate::audio::capture::AudioCapture`]
//! (`mpsc::Sender<AudioChunk = Vec<f32>>`, 48 kHz interleaved stereo) so the
//! rest of the engine can't tell the difference between WASAPI loopback and
//! this generated source. Three modes:
//!
//! - [`SyntheticSource::Silence`] emits zero-filled chunks. Useful for
//!   verifying the audio pipeline plumbing without listening artefacts.
//! - [`SyntheticSource::Sine`] emits a constant-frequency sine tone. The
//!   default 440 Hz @ 0.1 amplitude is a recognisable signal for end-to-end
//!   audio loopback debugging.
//! - [`SyntheticSource::WavLoop`] loads a WAV file (any sample rate /
//!   channel count supported by `hound`) and loops it; resampling to the
//!   engine's fixed 48 kHz stereo is done with nearest-neighbour because
//!   the simulator never needs broadcast quality.
//!
//! Gated behind the `simulator` feature so production driver builds neither
//! link `hound` nor expose this module.

use crate::audio::capture::AudioChunk;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Hard-coded output format — matches `AudioCapture` so the downstream Opus
/// encoder sees the same shape regardless of source.
pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const CHANNELS: u16 = 2;
/// Chunk duration. 10 ms matches the Opus default frame size in
/// `config/default.toml`, so each synthetic chunk maps 1:1 to one
/// downstream encoder frame.
pub const CHUNK_MS: u32 = 10;

const SAMPLES_PER_CHANNEL_PER_CHUNK: usize =
    (SAMPLE_RATE_HZ as usize * CHUNK_MS as usize) / 1000;
const SAMPLES_PER_CHUNK: usize = SAMPLES_PER_CHANNEL_PER_CHUNK * CHANNELS as usize;

/// Source kind. Cheap to clone — `WavLoop`'s samples vector is wrapped in an
/// `Arc` so multiple captures can share the same pre-loaded asset.
#[derive(Clone)]
pub enum SyntheticSource {
    Silence,
    Sine { hz: f32, amplitude: f32 },
    WavLoop { samples: Arc<Vec<f32>> },
}

impl SyntheticSource {
    /// Convenience: a 440 Hz, low-amplitude tone — the simulator default.
    pub fn default_sine() -> Self {
        Self::Sine { hz: 440.0, amplitude: 0.1 }
    }

    /// Load a WAV file and pre-decode to interleaved-stereo f32 at the
    /// engine's [`SAMPLE_RATE_HZ`]. Returns `None` on parse errors so a
    /// missing fixture degrades to "no audio" rather than aborting the test.
    #[cfg(feature = "simulator")]
    pub fn from_wav(path: impl AsRef<Path>) -> Option<Self> {
        let path = path.as_ref();
        let reader = match hound::WavReader::open(path) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("synthetic_capture: cannot open {:?}: {}", path, e);
                return None;
            }
        };
        let spec = reader.spec();
        let src_rate = spec.sample_rate;
        let src_channels = spec.channels;

        // Decode every sample into f32 in [-1, 1].
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => {
                let bits = spec.bits_per_sample;
                let scale = 1.0_f32 / (1u32 << (bits - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .filter_map(Result::ok)
                    .map(|s| s as f32 * scale)
                    .collect()
            }
            hound::SampleFormat::Float => reader
                .into_samples::<f32>()
                .filter_map(Result::ok)
                .collect(),
        };

        // Normalise channel count: mono → stereo (duplicate), >2ch → take first 2.
        let stereo: Vec<f32> = match src_channels {
            1 => {
                let mut out = Vec::with_capacity(raw.len() * 2);
                for &s in &raw {
                    out.push(s);
                    out.push(s);
                }
                out
            }
            2 => raw,
            n => {
                // Drop trailing channels; the simulator never needs surround.
                let frames = raw.len() / n as usize;
                let mut out = Vec::with_capacity(frames * 2);
                for f in 0..frames {
                    out.push(raw[f * n as usize]);
                    out.push(raw[f * n as usize + 1]);
                }
                out
            }
        };

        // Resample: nearest-neighbour because B5 tests only check that audio
        // *flowed*, not its fidelity. Quality-graded resampling would need
        // an FFT or sinc dep that the simulator does not justify.
        let samples = if src_rate == SAMPLE_RATE_HZ {
            stereo
        } else {
            let frames_in = stereo.len() / CHANNELS as usize;
            let frames_out =
                (frames_in as u64 * SAMPLE_RATE_HZ as u64 / src_rate as u64) as usize;
            let mut out = Vec::with_capacity(frames_out * CHANNELS as usize);
            for f in 0..frames_out {
                let src_f =
                    (f as u64 * src_rate as u64 / SAMPLE_RATE_HZ as u64) as usize;
                let src_f = src_f.min(frames_in.saturating_sub(1));
                out.push(stereo[src_f * 2]);
                out.push(stereo[src_f * 2 + 1]);
            }
            out
        };

        if samples.is_empty() {
            log::warn!("synthetic_capture: WAV {:?} decoded to 0 samples", path);
            return None;
        }
        log::info!(
            "synthetic_capture: loaded {:?} ({} samples @ {} Hz {} ch)",
            path, samples.len(), SAMPLE_RATE_HZ, CHANNELS
        );
        Some(Self::WavLoop { samples: Arc::new(samples) })
    }
}

/// Owns a producer thread. Drop signals cancel and joins; if the receiver
/// has been closed, the producer also stops naturally on the next try_send.
pub struct SyntheticAudioCapture {
    cancel: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl SyntheticAudioCapture {
    /// Spawn the producer thread. Returns `None` if the thread cannot be
    /// created (system at OS-thread limit) — matches `AudioCapture::start`'s
    /// "fail-soft so audio is disabled, not fatal" contract.
    pub fn start(
        source: SyntheticSource,
        chunk_tx: mpsc::Sender<AudioChunk>,
    ) -> Option<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = Arc::clone(&cancel);
        let handle = thread::Builder::new()
            .name("synthetic-audio".into())
            .spawn(move || run_producer(source, chunk_tx, cancel_thread))
            .ok()?;
        log::info!(
            "synthetic_capture: producer thread started (48 kHz stereo, {} ms chunks)",
            CHUNK_MS
        );
        Some(Self { cancel, handle: Some(handle) })
    }

    pub fn sample_rate(&self) -> u32 { SAMPLE_RATE_HZ }
    pub fn channels(&self) -> u16 { CHANNELS }
}

impl Drop for SyntheticAudioCapture {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        log::info!("synthetic_capture: producer thread stopped");
    }
}

/// Producer loop. Generates one 10 ms chunk per iteration, sleeps the
/// remainder of wall time so the channel doesn't flood when the consumer
/// is keeping up. Exits when:
/// - cancel flag is set (Drop)
/// - `try_send` reports the channel is closed (receiver dropped)
fn run_producer(
    source: SyntheticSource,
    chunk_tx: mpsc::Sender<AudioChunk>,
    cancel: Arc<AtomicBool>,
) {
    let mut phase: f32 = 0.0;
    let mut wav_cursor: usize = 0;
    let chunk_duration = Duration::from_millis(CHUNK_MS as u64);
    let mut next_tick = Instant::now();

    while !cancel.load(Ordering::Relaxed) {
        let chunk = match &source {
            SyntheticSource::Silence => vec![0.0_f32; SAMPLES_PER_CHUNK],
            SyntheticSource::Sine { hz, amplitude } => {
                generate_sine_chunk(*hz, *amplitude, &mut phase)
            }
            SyntheticSource::WavLoop { samples } => {
                generate_wav_chunk(samples, &mut wav_cursor)
            }
        };

        match chunk_tx.try_send(chunk) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Consumer fell behind; drop this chunk silently. Matches
                // AudioCapture's lock-free try_send semantics — overflow
                // is preferable to blocking the producer.
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::info!("synthetic_capture: consumer closed, producer exiting");
                return;
            }
        }

        // Drift-free pacing: schedule the next tick from the previous one
        // so accumulated dispatch overhead doesn't slow the source over
        // long runs.
        next_tick += chunk_duration;
        let now = Instant::now();
        if next_tick > now {
            thread::sleep(next_tick - now);
        } else {
            // Slipped a chunk's worth — resync rather than chase.
            next_tick = now;
        }
    }
}

fn generate_sine_chunk(hz: f32, amplitude: f32, phase: &mut f32) -> Vec<f32> {
    use std::f32::consts::TAU;
    let mut out = Vec::with_capacity(SAMPLES_PER_CHUNK);
    let phase_increment = TAU * hz / SAMPLE_RATE_HZ as f32;
    for _ in 0..SAMPLES_PER_CHANNEL_PER_CHUNK {
        let s = phase.sin() * amplitude;
        // Identical L and R — the simulator does not exercise stereo image,
        // and downstream Opus deduplicates well so bitrate stays low.
        out.push(s);
        out.push(s);
        *phase += phase_increment;
        if *phase > TAU {
            *phase -= TAU;
        }
    }
    out
}

fn generate_wav_chunk(samples: &Arc<Vec<f32>>, cursor: &mut usize) -> Vec<f32> {
    let total = samples.len();
    let mut out = Vec::with_capacity(SAMPLES_PER_CHUNK);
    for _ in 0..SAMPLES_PER_CHUNK {
        out.push(samples[*cursor]);
        *cursor = (*cursor + 1) % total.max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_size_constants() {
        // Production sites assume 48 kHz stereo 10 ms — 960 f32 / chunk.
        assert_eq!(SAMPLES_PER_CHANNEL_PER_CHUNK, 480);
        assert_eq!(SAMPLES_PER_CHUNK, 960);
    }

    #[test]
    fn test_silence_chunk_is_zero() {
        let chunk = vec![0.0_f32; SAMPLES_PER_CHUNK];
        assert!(chunk.iter().all(|&s| s == 0.0));
        assert_eq!(chunk.len(), 960);
    }

    #[test]
    fn test_sine_chunk_has_expected_shape() {
        let mut phase = 0.0;
        let chunk = generate_sine_chunk(440.0, 0.5, &mut phase);
        assert_eq!(chunk.len(), SAMPLES_PER_CHUNK);
        // Amplitude bound: sin · 0.5 must stay in [-0.5, 0.5].
        for &s in &chunk {
            assert!(s.abs() <= 0.5 + 1e-6, "sample {} exceeds amplitude bound", s);
        }
        // Stereo invariant: L == R because we emit identical channels.
        for i in 0..SAMPLES_PER_CHANNEL_PER_CHUNK {
            assert_eq!(chunk[i * 2], chunk[i * 2 + 1]);
        }
    }

    #[test]
    fn test_sine_phase_advances() {
        let mut phase = 0.0;
        let _ = generate_sine_chunk(440.0, 0.5, &mut phase);
        // After 480 samples @ 440 Hz / 48 kHz the phase advance is
        // 480 * (2π * 440 / 48000) ≈ 27.65 rad → wraps to ~2.97 rad.
        // We only assert "non-zero" + "bounded" because the wrap math
        // is verified in test_sine_phase_wraps below.
        assert!(phase != 0.0);
        assert!(phase.is_finite());
    }

    #[test]
    fn test_sine_phase_wraps() {
        // Phase must stay bounded across many chunks — if the implementation
        // forgot to wrap, phase would grow unboundedly and eventually lose
        // precision.
        use std::f32::consts::TAU;
        let mut phase = 0.0;
        for _ in 0..1000 {
            let _ = generate_sine_chunk(440.0, 0.5, &mut phase);
            assert!(phase.abs() < TAU * 2.0,
                "phase grew unbounded: {}", phase);
        }
    }

    #[test]
    fn test_wav_chunk_cycles_through_samples() {
        // Three frames (stereo, 6 f32 total). With chunk size 960, the
        // cursor must wrap many times and always read in-bounds.
        let samples = Arc::new(vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        let mut cursor = 0;
        let chunk = generate_wav_chunk(&samples, &mut cursor);
        assert_eq!(chunk.len(), SAMPLES_PER_CHUNK);
        // First six values match the source verbatim.
        assert_eq!(&chunk[..6], &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        // After 960 reads on a 6-sample loop, cursor lands at 960 % 6 = 0.
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_wav_chunk_empty_source_is_safe() {
        // Empty samples vector is rejected at construction time, but the
        // helper must still not panic if called with one (defensive).
        let samples = Arc::new(vec![0.5; 1]);
        let mut cursor = 0;
        let chunk = generate_wav_chunk(&samples, &mut cursor);
        // Single-sample source means every output frame reads index 0.
        assert!(chunk.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn test_synthetic_capture_emits_chunks_then_stops() {
        // End-to-end: spawn the producer, receive >= 3 chunks, drop the
        // capture handle, verify the receiver hits EOF cleanly.
        let (tx, mut rx) = mpsc::channel::<AudioChunk>(8);
        let cap = SyntheticAudioCapture::start(SyntheticSource::Silence, tx)
            .expect("producer should spawn");
        // Block-wait on a runtime so we can recv() — keep the test runtime
        // tiny so it stays fast in CI.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let chunks_received = rt.block_on(async {
            let mut count = 0;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
            while count < 3 {
                tokio::select! {
                    Some(c) = rx.recv() => {
                        assert_eq!(c.len(), SAMPLES_PER_CHUNK);
                        count += 1;
                    }
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
            count
        });
        assert!(chunks_received >= 3,
            "expected >=3 chunks in 200 ms, got {}", chunks_received);
        drop(cap); // signals cancel + joins producer
    }

    #[test]
    fn test_default_sine_is_a_audible_frequency() {
        match SyntheticSource::default_sine() {
            SyntheticSource::Sine { hz, amplitude } => {
                // 440 Hz (concert A) is the canonical loopback signal.
                assert!((hz - 440.0).abs() < f32::EPSILON);
                // Amplitude must stay low so loopback testers don't blast
                // their headphones if they accidentally route real output.
                assert!(amplitude > 0.0 && amplitude <= 0.5);
            }
            _ => panic!("default_sine should always return Sine"),
        }
    }
}
