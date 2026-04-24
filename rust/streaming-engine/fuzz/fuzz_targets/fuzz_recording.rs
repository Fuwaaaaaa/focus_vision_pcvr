#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::sync::atomic::{AtomicU64, Ordering};
use streaming_engine::recording::{AudioRecorder, Recorder};

#[derive(Debug, Arbitrary)]
struct RecInput {
    /// Sequence of NAL-like byte buffers fed into Recorder.
    nals: Vec<Vec<u8>>,
    /// PCM samples for AudioRecorder. Bit patterns are reinterpreted as f32.
    pcm_bytes: Vec<u8>,
}

/// Monotonic counter for unique temp filenames — avoids collisions when the
/// fuzzer spawns workers in parallel.
static COUNTER: AtomicU64 = AtomicU64::new(0);

fuzz_target!(|input: RecInput| {
    // Cap total input size to keep per-iteration cost bounded (prevents OOM on
    // pathological inputs; real recordings are much smaller per iteration).
    let total_nal_bytes: usize = input.nals.iter().map(|n| n.len()).sum();
    if total_nal_bytes > 256 * 1024 {
        return;
    }
    if input.pcm_bytes.len() > 64 * 1024 {
        return;
    }

    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    // Video recorder: feed arbitrary NAL buffers.
    let video_path = std::env::temp_dir().join(format!("fvp_fuzz_rec_v_{pid}_{id}.h265"));
    if let Some(mut rec) = Recorder::open(&video_path) {
        for nal in &input.nals {
            rec.write_nal(nal);
        }
        // rec drops here → close() flushes and logs
    }
    let _ = std::fs::remove_file(&video_path);

    // Audio recorder: reinterpret byte buffer as f32 samples.
    let audio_path = std::env::temp_dir().join(format!("fvp_fuzz_rec_a_{pid}_{id}.wav"));
    if let Some(mut rec) = AudioRecorder::open(&audio_path, 48000, 2) {
        // 4 bytes per f32; truncate to multiple of 4
        let usable = input.pcm_bytes.len() & !3;
        let samples: Vec<f32> = input.pcm_bytes[..usable]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        rec.write_pcm_f32(&samples);
    }
    let _ = std::fs::remove_file(&audio_path);
});
