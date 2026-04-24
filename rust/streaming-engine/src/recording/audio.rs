//! Audio session recording as 16-bit PCM WAV.
//!
//! Writes a minimal RIFF/WAVE header up-front with placeholder sizes, then
//! appends PCM samples as they arrive. On Drop/close, seeks back and patches
//! in the final data and RIFF chunk sizes so the file is valid.
//!
//! Converts incoming f32 samples to i16 (×32767 saturation) for maximum
//! compatibility; any ordinary player (VLC, Audacity, Windows Media Player,
//! QuickTime) can play the result.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub struct AudioRecorder {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    sample_rate: u32,
    channels: u16,
    /// Count of PCM bytes written (excluding header).
    data_bytes: u32,
    poisoned: bool,
}

impl AudioRecorder {
    /// Open a WAV file at `path`. The header is pre-written with placeholder
    /// sizes; they are patched on close/drop.
    pub fn open(path: impl Into<PathBuf>, sample_rate: u32, channels: u16) -> Option<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("audio recorder: cannot create dir {:?}: {}", parent, e);
                return None;
            }
        }
        let file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("audio recorder: cannot open {:?}: {}", path, e);
                return None;
            }
        };
        let mut rec = Self {
            path,
            writer: Some(BufWriter::with_capacity(64 * 1024, file)),
            sample_rate,
            channels,
            data_bytes: 0,
            poisoned: false,
        };
        if !rec.write_header() {
            return None;
        }
        log::info!("audio recording started → {:?} ({} Hz {}ch)",
            rec.path, sample_rate, channels);
        Some(rec)
    }

    fn write_header(&mut self) -> bool {
        let w = match self.writer.as_mut() {
            Some(w) => w,
            None => return false,
        };
        let bits_per_sample: u16 = 16;
        let byte_rate: u32 = self.sample_rate * self.channels as u32 * (bits_per_sample / 8) as u32;
        let block_align: u16 = self.channels * (bits_per_sample / 8);
        // RIFF chunk + WAVE id + fmt subchunk + data subchunk header = 44 bytes
        let mut hdr = Vec::with_capacity(44);
        hdr.extend_from_slice(b"RIFF");
        hdr.extend_from_slice(&0u32.to_le_bytes());   // placeholder: RIFF size
        hdr.extend_from_slice(b"WAVE");
        hdr.extend_from_slice(b"fmt ");
        hdr.extend_from_slice(&16u32.to_le_bytes());  // fmt chunk size
        hdr.extend_from_slice(&1u16.to_le_bytes());   // PCM format
        hdr.extend_from_slice(&self.channels.to_le_bytes());
        hdr.extend_from_slice(&self.sample_rate.to_le_bytes());
        hdr.extend_from_slice(&byte_rate.to_le_bytes());
        hdr.extend_from_slice(&block_align.to_le_bytes());
        hdr.extend_from_slice(&bits_per_sample.to_le_bytes());
        hdr.extend_from_slice(b"data");
        hdr.extend_from_slice(&0u32.to_le_bytes());   // placeholder: data size
        if let Err(e) = w.write_all(&hdr) {
            log::warn!("audio recorder: header write failed: {}", e);
            self.poisoned = true;
            return false;
        }
        true
    }

    /// Append interleaved f32 samples. Values outside [-1.0, 1.0] are clipped.
    pub fn write_pcm_f32(&mut self, samples: &[f32]) {
        if self.poisoned || samples.is_empty() {
            return;
        }
        let w = match self.writer.as_mut() {
            Some(w) => w,
            None => return,
        };
        // Convert f32 → i16 with saturation, write little-endian.
        // 4 KB chunk avoids large intermediate allocations.
        const CHUNK: usize = 1024;
        let mut buf = [0u8; CHUNK * 2];
        for block in samples.chunks(CHUNK) {
            let mut off = 0;
            for &s in block {
                let clamped = s.clamp(-1.0, 1.0);
                let i = (clamped * 32767.0) as i16;
                let bytes = i.to_le_bytes();
                buf[off] = bytes[0];
                buf[off + 1] = bytes[1];
                off += 2;
            }
            if let Err(e) = w.write_all(&buf[..off]) {
                log::warn!("audio recorder: write error: {}", e);
                self.poisoned = true;
                return;
            }
            self.data_bytes = self.data_bytes.saturating_add(off as u32);
        }
    }

    /// Patch header sizes and close the file.
    pub fn close(&mut self) {
        if let Some(mut w) = self.writer.take() {
            if let Err(e) = w.flush() {
                log::warn!("audio recorder: flush error: {}", e);
                return;
            }
            // RIFF chunk size = total file size - 8 (the "RIFF" + size fields).
            // Header is 44 bytes, so RIFF size = 44 - 8 + data_bytes = 36 + data_bytes.
            let riff_size = 36u32.saturating_add(self.data_bytes);
            // Recover the underlying file and seek-patch the two size fields.
            let mut file = match w.into_inner() {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("audio recorder: into_inner failed: {}", e);
                    return;
                }
            };
            // RIFF size at offset 4
            if file.seek(SeekFrom::Start(4)).is_ok() {
                let _ = file.write_all(&riff_size.to_le_bytes());
            }
            // data size at offset 40 (8 RIFF + 4 WAVE + 8 fmt hdr + 16 fmt + 4 "data")
            if file.seek(SeekFrom::Start(40)).is_ok() {
                let _ = file.write_all(&self.data_bytes.to_le_bytes());
            }
            log::info!("audio recording closed → {:?} ({} data bytes)",
                self.path, self.data_bytes);
        }
    }

    pub fn path(&self) -> &Path { &self.path }
    pub fn data_bytes(&self) -> u32 { self.data_bytes }
    pub fn is_open(&self) -> bool { self.writer.is_some() && !self.poisoned }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        self.close();
    }
}

/// Default .wav filename with UTC timestamp.
/// Example: `recording_2026-04-24T01-59-03.wav`
pub fn default_audio_filename() -> String {
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    format!("recording_{}.wav", ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fvp_arec_test_{}.wav", name))
    }

    fn read_u32_le(data: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
    }

    #[test]
    fn test_empty_wav_header_valid() {
        let p = temp_path("empty");
        let _ = fs::remove_file(&p);
        {
            let _rec = AudioRecorder::open(&p, 48000, 2).unwrap();
        }
        let data = fs::read(&p).unwrap();
        assert_eq!(data.len(), 44);
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..12], b"WAVE");
        assert_eq!(&data[12..16], b"fmt ");
        assert_eq!(read_u32_le(&data, 16), 16); // fmt size
        assert_eq!(u16::from_le_bytes([data[20], data[21]]), 1); // PCM
        assert_eq!(u16::from_le_bytes([data[22], data[23]]), 2); // channels
        assert_eq!(read_u32_le(&data, 24), 48000); // sample rate
        assert_eq!(&data[36..40], b"data");
        assert_eq!(read_u32_le(&data, 40), 0); // data size
        assert_eq!(read_u32_le(&data, 4), 36); // RIFF size = 36 + 0
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_write_pcm_updates_sizes() {
        let p = temp_path("write");
        let _ = fs::remove_file(&p);
        {
            let mut rec = AudioRecorder::open(&p, 48000, 2).unwrap();
            rec.write_pcm_f32(&[0.0, 0.5, -0.5, 1.0]); // 4 f32 → 8 bytes i16
            assert_eq!(rec.data_bytes(), 8);
        }
        let data = fs::read(&p).unwrap();
        assert_eq!(data.len(), 44 + 8);
        assert_eq!(read_u32_le(&data, 40), 8);
        assert_eq!(read_u32_le(&data, 4), 36 + 8);
        // sample 1: 0.5 * 32767 = 16383 (LE: 0xFF 0x3F)
        assert_eq!(&data[44..46], &[0x00, 0x00]);
        assert_eq!(&data[46..48], &[0xFF, 0x3F]);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_clipping() {
        let p = temp_path("clip");
        let _ = fs::remove_file(&p);
        {
            let mut rec = AudioRecorder::open(&p, 48000, 1).unwrap();
            rec.write_pcm_f32(&[2.0, -2.0, f32::NAN]); // NaN clamps to 0 via sign? actually NaN comparisons are false
        }
        let data = fs::read(&p).unwrap();
        // 2.0 → clamp to 1.0 → 32767 (0x7FFF LE)
        // -2.0 → clamp to -1.0 → -32767 (0x8001 LE)
        // NaN → clamp returns NaN, `as i16` yields 0 on NaN
        assert_eq!(&data[44..46], &[0xFF, 0x7F]);
        assert_eq!(&data[46..48], &[0x01, 0x80]);
        assert_eq!(&data[48..50], &[0x00, 0x00]);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_default_audio_filename() {
        let name = default_audio_filename();
        assert!(name.starts_with("recording_"));
        assert!(name.ends_with(".wav"));
        assert_eq!(name.len(), 10 + 19 + 4);
    }
}
