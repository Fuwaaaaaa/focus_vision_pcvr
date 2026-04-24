//! Session recording: writes raw Annex B H.264/H.265 NAL streams to disk.
//!
//! Design: one file per session, written as NAL units prefixed with the
//! Annex B start code (`00 00 00 01`) when the caller hasn't already
//! emitted one. VLC, ffmpeg, and most media tools can play Annex B directly;
//! to convert to mp4: `ffmpeg -i rec.h265 -c copy rec.mp4`.
//!
//! The recorder is best-effort — I/O errors are logged but never propagated
//! to the streaming pipeline. A recording that starts failing mid-session
//! simply stops writing; the stream continues uninterrupted.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Annex B start code prefix (4-byte variant). Precedes every NAL unit.
const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Raw Annex B NAL stream writer.
///
/// Call `write_nal()` per encoded frame / NAL. Drop or call `close()` to
/// flush buffered bytes. Not `Clone` / `Send` — wrap in `Arc<Mutex<_>>` at
/// the call site if multiple producers write.
pub struct Recorder {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    /// Total bytes written since open (for diagnostics).
    bytes_written: u64,
    /// NAL units written.
    nal_count: u64,
    /// Once an I/O error occurs, stop attempting further writes.
    poisoned: bool,
}

impl Recorder {
    /// Open a new recording at `path`. Creates parent directories as needed.
    /// Returns None if the file cannot be created.
    pub fn open(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("recorder: cannot create dir {:?}: {}", parent, e);
                return None;
            }
        }
        let file = match File::create(&path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("recorder: cannot open {:?}: {}", path, e);
                return None;
            }
        };
        log::info!("recording started → {:?}", path);
        Some(Self {
            path,
            writer: Some(BufWriter::with_capacity(256 * 1024, file)),
            bytes_written: 0,
            nal_count: 0,
            poisoned: false,
        })
    }

    /// Write one NAL unit. If `nal` already begins with an Annex B start
    /// code (3- or 4-byte), it is passed through unmodified. Otherwise a
    /// 4-byte start code is prepended.
    pub fn write_nal(&mut self, nal: &[u8]) {
        if self.poisoned || nal.is_empty() {
            return;
        }
        let writer = match self.writer.as_mut() {
            Some(w) => w,
            None => return,
        };

        let has_start_code = starts_with_annexb(nal);
        let res = if has_start_code {
            writer.write_all(nal).map(|_| nal.len())
        } else {
            writer
                .write_all(&START_CODE)
                .and_then(|_| writer.write_all(nal))
                .map(|_| nal.len() + START_CODE.len())
        };
        match res {
            Ok(n) => {
                self.bytes_written += n as u64;
                self.nal_count += 1;
            }
            Err(e) => {
                log::warn!("recorder: write error ({} bytes total, poisoning): {}",
                    self.bytes_written, e);
                self.poisoned = true;
            }
        }
    }

    /// Flush buffered bytes. Errors are logged only.
    pub fn flush(&mut self) {
        if let Some(w) = self.writer.as_mut() {
            if let Err(e) = w.flush() {
                log::warn!("recorder: flush error: {}", e);
            }
        }
    }

    /// Close the file and emit a summary log line. Also called automatically on Drop.
    pub fn close(&mut self) {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
            log::info!(
                "recording closed → {:?} ({} NALs, {} bytes)",
                self.path, self.nal_count, self.bytes_written
            );
        }
    }

    pub fn path(&self) -> &Path { &self.path }
    pub fn bytes_written(&self) -> u64 { self.bytes_written }
    pub fn nal_count(&self) -> u64 { self.nal_count }
    pub fn is_open(&self) -> bool { self.writer.is_some() && !self.poisoned }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.close();
    }
}

/// True if `buf` starts with an Annex B start code (3- or 4-byte variant).
fn starts_with_annexb(buf: &[u8]) -> bool {
    matches!(buf, [0x00, 0x00, 0x00, 0x01, ..] | [0x00, 0x00, 0x01, ..])
}

/// Generate a default recording filename with the current UTC timestamp.
/// Example: `recording_2026-04-24T01-59-03.h265`
pub fn default_filename(codec_ext: &str) -> String {
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S");
    format!("recording_{}.{}", ts, codec_ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("fvp_rec_test_{}.h265", name))
    }

    #[test]
    fn test_starts_with_annexb_variants() {
        assert!(starts_with_annexb(&[0x00, 0x00, 0x00, 0x01, 0x67]));
        assert!(starts_with_annexb(&[0x00, 0x00, 0x01, 0x67]));
        assert!(!starts_with_annexb(&[0x00, 0x00, 0x00]));
        assert!(!starts_with_annexb(&[0x67, 0x42]));
        assert!(!starts_with_annexb(&[]));
    }

    #[test]
    fn test_write_nal_prepends_start_code() {
        let p = temp_path("prepend");
        let _ = fs::remove_file(&p);
        {
            let mut rec = Recorder::open(&p).unwrap();
            rec.write_nal(&[0x67, 0x42, 0x00]); // raw NAL without start code
            rec.close();
        }
        let data = fs::read(&p).unwrap();
        assert_eq!(&data, &[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00]);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_write_nal_passes_existing_start_code() {
        let p = temp_path("passthru");
        let _ = fs::remove_file(&p);
        {
            let mut rec = Recorder::open(&p).unwrap();
            rec.write_nal(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42]);
            rec.write_nal(&[0x00, 0x00, 0x01, 0x68]);
            rec.close();
        }
        let data = fs::read(&p).unwrap();
        assert_eq!(
            &data,
            &[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x00, 0x01, 0x68]
        );
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_empty_nal_is_noop() {
        let p = temp_path("empty");
        let _ = fs::remove_file(&p);
        {
            let mut rec = Recorder::open(&p).unwrap();
            rec.write_nal(&[]);
            assert_eq!(rec.nal_count(), 0);
            assert_eq!(rec.bytes_written(), 0);
        }
        let data = fs::read(&p).unwrap();
        assert!(data.is_empty());
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_drop_flushes() {
        let p = temp_path("drop");
        let _ = fs::remove_file(&p);
        {
            let mut rec = Recorder::open(&p).unwrap();
            rec.write_nal(&[0x67, 0x42]);
            // implicit drop
        }
        let data = fs::read(&p).unwrap();
        assert_eq!(data.len(), 4 + 2);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn test_open_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("fvp_rec_test_nested").join("inner");
        let _ = fs::remove_dir_all(dir.parent().unwrap());
        let p = dir.join("x.h265");
        {
            let mut rec = Recorder::open(&p).unwrap();
            rec.write_nal(&[0x01]);
        }
        assert!(p.exists());
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn test_default_filename_format() {
        let name = default_filename("h265");
        assert!(name.starts_with("recording_"));
        assert!(name.ends_with(".h265"));
        // length = "recording_" (10) + "YYYY-MM-DDTHH-MM-SS" (19) + ".h265" (5)
        assert_eq!(name.len(), 10 + 19 + 5);
    }

    #[test]
    fn test_counts_update() {
        let p = temp_path("counts");
        let _ = fs::remove_file(&p);
        let mut rec = Recorder::open(&p).unwrap();
        rec.write_nal(&[0x67, 0x42]); // 2 + 4 start = 6 bytes
        rec.write_nal(&[0x68]);        // 1 + 4 = 5
        assert_eq!(rec.nal_count(), 2);
        assert_eq!(rec.bytes_written(), 11);
        let _ = fs::remove_file(&p);
    }
}
