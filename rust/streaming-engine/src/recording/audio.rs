//! Audio session recording as 16-bit PCM WAV.
//!
//! Writes a minimal RIFF/WAVE header up-front with placeholder sizes, then
//! appends PCM samples as they arrive. Every second of audio, and on
//! Drop/close, it seeks back and patches in the data and RIFF chunk sizes,
//! so the file is valid up to the last second even if the process dies
//! (vrserver.exe crashing or being killed) before it closes.
//!
//! Converts incoming f32 samples to i16 (×32767 saturation) for maximum
//! compatibility; any ordinary player (VLC, Audacity, Windows Media Player,
//! QuickTime) can play the result.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The most PCM a WAV can hold: its sizes are u32, and the RIFF size is the
/// data's plus 36. About 6 hours of 48 kHz stereo.
const MAX_DATA_BYTES: u32 = u32::MAX - 36;

pub struct AudioRecorder {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    sample_rate: u32,
    channels: u16,
    /// Count of PCM bytes written (excluding header).
    data_bytes: u32,
    /// `data_bytes` when the header's sizes were last written.
    patched_bytes: u32,
    /// Where the recording stops (`MAX_DATA_BYTES`; lower in tests).
    max_data_bytes: u32,
    poisoned: bool,
}

impl AudioRecorder {
    /// Open a WAV file at `path` (or next to it, with a `-2`… suffix, if that
    /// file exists: see [`Self::path`]). The header is pre-written with
    /// placeholder sizes; they are patched every second and on close/drop.
    pub fn open(path: impl Into<PathBuf>, sample_rate: u32, channels: u16) -> Option<Self> {
        let path = path.into();
        let (file, path) = match super::create_new_file(&path) {
            Ok(created) => created,
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
            patched_bytes: 0,
            max_data_bytes: MAX_DATA_BYTES,
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
        if self.poisoned || samples.is_empty() || self.writer.is_none() {
            return;
        }
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
            // REGRESSION: past 4 GB the sizes saturated and the header no
            // longer described the file.
            if off as u32 > self.max_data_bytes - self.data_bytes {
                log::warn!("audio recording reached the WAV size limit ({} bytes, about 6 h at 48 kHz stereo); stopped → {:?}",
                    self.data_bytes, self.path);
                self.close();
                return;
            }
            let Some(w) = self.writer.as_mut() else { return };
            if let Err(e) = w.write_all(&buf[..off]) {
                log::warn!("audio recorder: write error: {}", e);
                self.poisoned = true;
                return;
            }
            self.data_bytes += off as u32;
        }
        // REGRESSION: the sizes were written only on close, so a recording
        // whose process died (vrserver.exe crashing or killed) read as empty.
        let second = self.sample_rate.saturating_mul(u32::from(self.channels)).saturating_mul(2);
        if self.data_bytes - self.patched_bytes >= second {
            self.patch_sizes();
        }
    }

    /// Write the header's sizes for what has been written so far, leaving
    /// the file positioned at its end.
    fn patch_sizes(&mut self) {
        let Some(w) = self.writer.as_mut() else { return };
        // RIFF chunk size = total file size - 8 (the "RIFF" + size fields).
        // Header is 44 bytes, so RIFF size = 44 - 8 + data_bytes = 36 + data_bytes.
        let riff_size = 36 + self.data_bytes;
        let result = w.flush().and_then(|()| {
            let file = w.get_mut();
            // RIFF size at offset 4
            file.seek(SeekFrom::Start(4))?;
            file.write_all(&riff_size.to_le_bytes())?;
            // data size at offset 40 (8 RIFF + 4 WAVE + 8 fmt hdr + 16 fmt + 4 "data")
            file.seek(SeekFrom::Start(40))?;
            file.write_all(&self.data_bytes.to_le_bytes())?;
            file.seek(SeekFrom::End(0)).map(|_| ())
        });
        match result {
            Ok(()) => self.patched_bytes = self.data_bytes,
            Err(e) => {
                log::warn!("audio recorder: header update failed: {}", e);
                self.poisoned = true;
            }
        }
    }

    /// Patch header sizes and close the file.
    pub fn close(&mut self) {
        if self.writer.is_none() {
            return;
        }
        self.patch_sizes();
        self.writer = None;
        log::info!("audio recording closed → {:?} ({} data bytes)",
            self.path, self.data_bytes);
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
    fn test_sizes_are_kept_current_for_a_process_that_dies() {
        // REGRESSION: sizes were written only on close, so vrserver.exe
        // crashing left a WAV that read as empty.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("crash.wav");
        let mut rec = AudioRecorder::open(&p, 48000, 2).unwrap();
        let frame = vec![0.25f32; 960]; // 10 ms of 48 kHz stereo
        for _ in 0..150 {
            rec.write_pcm_f32(&frame);
        }
        // The process dies: the buffered samples may still reach the file,
        // the closing header update never happens.
        drop(rec.writer.take());
        drop(rec);

        let data = fs::read(&p).unwrap();
        let one_second = 48000 * 2 * 2;
        assert_eq!(data.len(), 44 + one_second as usize * 3 / 2);
        assert_eq!(read_u32_le(&data, 40), one_second, "the header covers each whole second written");
        assert_eq!(read_u32_le(&data, 4), 36 + one_second);
    }

    #[test]
    fn test_recording_stops_at_the_wav_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("full.wav");
        {
            let mut rec = AudioRecorder::open(&p, 48000, 1).unwrap();
            rec.max_data_bytes = 100;
            rec.write_pcm_f32(&[0.5; 40]); // 80 bytes
            assert!(rec.is_open());
            rec.write_pcm_f32(&[0.5; 20]); // 40 more would pass 100
            assert!(!rec.is_open(), "stopped at the limit");
            assert_eq!(rec.data_bytes(), 80);
            rec.write_pcm_f32(&[0.5; 1]);
            assert_eq!(rec.data_bytes(), 80, "nothing after the stop");
        }
        let data = fs::read(&p).unwrap();
        assert_eq!(data.len(), 44 + 80);
        assert_eq!(read_u32_le(&data, 40), 80);
    }

    #[test]
    fn test_same_second_session_keeps_the_previous_recording() {
        // REGRESSION: File::create truncated the file a session reconnecting
        // in the same second had just written.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("recording_2026-04-24T01-59-03.wav");
        {
            let mut first = AudioRecorder::open(&p, 48000, 2).unwrap();
            first.write_pcm_f32(&[0.5; 960]);
        }
        let second = AudioRecorder::open(&p, 48000, 2).unwrap();
        assert_eq!(second.path(), dir.path().join("recording_2026-04-24T01-59-03-2.wav"));
        assert_eq!(read_u32_le(&fs::read(&p).unwrap(), 40), 1920, "the first recording is intact");
    }

    #[test]
    fn test_default_audio_filename() {
        let name = default_audio_filename();
        assert!(name.starts_with("recording_"));
        assert!(name.ends_with(".wav"));
        assert_eq!(name.len(), 10 + 19 + 4);
    }
}
