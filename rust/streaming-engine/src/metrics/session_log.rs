use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// A single session log record, written as one JSONL line every 10 seconds.
#[derive(Debug, Clone, Serialize)]
pub struct SessionRecord {
    pub ts: String,
    pub latency_ms: u32,
    pub bitrate_mbps: u32,
    pub loss_pct: f32,
    pub fec_pct: f32,
    pub fps: u16,
    pub encoder_resets: u32,
}

/// Session logger that buffers records and flushes to disk periodically.
///
/// - Records are buffered in memory (60 seconds worth)
/// - Flushed to a JSONL file on disk when buffer is full or on drop
/// - Log files are rotated: files older than `retention_days` are deleted
pub struct SessionLogger {
    dir: PathBuf,
    file_path: PathBuf,
    buffer: Vec<String>,
    last_flush: Instant,
    flush_interval: Duration,
    retention_days: u32,
}

impl SessionLogger {
    /// Create a new session logger. Creates the directory if it doesn't exist.
    pub fn new(dir: &Path, retention_days: u32) -> Result<Self, std::io::Error> {
        fs::create_dir_all(dir)?;

        let now = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let file_name = format!("session_{}.jsonl", now);
        let file_path = dir.join(file_name);

        Ok(Self {
            dir: dir.to_path_buf(),
            file_path,
            buffer: Vec::with_capacity(6), // ~60s at 10s intervals
            last_flush: Instant::now(),
            flush_interval: Duration::from_secs(60),
            retention_days,
        })
    }

    /// Record a session data point. Flushes to disk if buffer interval has elapsed.
    pub fn record(&mut self, record: SessionRecord) {
        if let Ok(json) = serde_json::to_string(&record) {
            self.buffer.push(json);
        }

        if self.last_flush.elapsed() >= self.flush_interval {
            self.flush();
        }
    }

    /// Flush buffered records to the JSONL file.
    pub fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        // Join all buffered records into a single write to minimize syscalls.
        let mut payload = String::with_capacity(
            self.buffer.iter().map(|l| l.len() + 1).sum(),
        );
        for line in &self.buffer {
            payload.push_str(line);
            payload.push('\n');
        }

        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(payload.as_bytes()) {
                    log::warn!("Session log write error: {}", e);
                    return;
                }
                self.buffer.clear();
                self.last_flush = Instant::now();
            }
            Err(e) => {
                log::warn!("Session log open error: {} — skipping flush", e);
            }
        }
    }

    /// Delete log files older than retention_days.
    pub fn rotate(&self) {
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(self.retention_days as u64 * 86400))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Session log rotation: can't read dir: {}", e);
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        if let Err(e) = fs::remove_file(&path) {
                            log::warn!("Session log rotation: can't delete {:?}: {}", path, e);
                        } else {
                            log::info!("Session log rotated: {:?}", path);
                        }
                    }
                }
            }
        }
    }

    /// Get the path to the current log file.
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }
}

impl Drop for SessionLogger {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn utc_timestamp() -> String {
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    fn make_record() -> SessionRecord {
        SessionRecord {
            ts: "2026-04-09T12:00:00Z".into(),
            latency_ms: 35,
            bitrate_mbps: 80,
            loss_pct: 0.5,
            fec_pct: 15.0,
            fps: 90,
            encoder_resets: 0,
        }
    }

    #[test]
    fn test_session_logger_creates_dir() {
        let dir = std::env::temp_dir().join("fvp_test_session_log_create");
        let _ = fs::remove_dir_all(&dir);
        let logger = SessionLogger::new(&dir, 7);
        assert!(logger.is_ok());
        assert!(dir.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_session_logger_write_and_flush() {
        let dir = std::env::temp_dir().join("fvp_test_session_log_write");
        let _ = fs::remove_dir_all(&dir);
        let mut logger = SessionLogger::new(&dir, 7).unwrap();

        logger.record(make_record());
        logger.record(make_record());
        logger.flush();

        let content = fs::read_to_string(logger.file_path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Verify JSON structure
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed["latency_ms"], 35);
        assert_eq!(parsed["fps"], 90);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_session_logger_drop_flushes() {
        let dir = std::env::temp_dir().join("fvp_test_session_log_drop");
        let _ = fs::remove_dir_all(&dir);
        let file_path;
        {
            let mut logger = SessionLogger::new(&dir, 7).unwrap();
            logger.record(make_record());
            file_path = logger.file_path().to_path_buf();
            // Drop logger — should flush
        }
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.lines().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_session_logger_empty_flush_noop() {
        let dir = std::env::temp_dir().join("fvp_test_session_log_empty");
        let _ = fs::remove_dir_all(&dir);
        let mut logger = SessionLogger::new(&dir, 7).unwrap();
        logger.flush(); // Should not create file
        assert!(!logger.file_path().exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_utc_timestamp_format() {
        let ts = utc_timestamp();
        // Should look like "2026-04-09T12:00:00Z"
        assert!(ts.contains('T'));
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        // Verify year is reasonable
        let year: u32 = ts[0..4].parse().unwrap();
        assert!(year >= 2024 && year <= 2100);
    }

    #[test]
    fn test_session_logger_rotate_preserves_recent_files() {
        // Test that rotate() doesn't delete recent files
        let dir = std::env::temp_dir().join("fvp_test_session_log_rotate");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create a fresh .jsonl file (mtime = now)
        let recent_file = dir.join("session_recent.jsonl");
        fs::write(&recent_file, "recent data\n").unwrap();

        // Create a non-jsonl file (should be ignored)
        let txt_file = dir.join("notes.txt");
        fs::write(&txt_file, "not a log\n").unwrap();

        let logger = SessionLogger::new(&dir, 7).unwrap();
        logger.rotate();

        // Recent .jsonl file should be preserved (mtime < 7 days old)
        assert!(recent_file.exists(), "Recent jsonl file should be preserved");
        // Non-jsonl file should be untouched
        assert!(txt_file.exists(), "Non-jsonl file should be ignored");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_session_logger_rotate_skips_non_jsonl() {
        let dir = std::env::temp_dir().join("fvp_test_session_log_rotate_ext");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let txt_file = dir.join("old.txt");
        fs::write(&txt_file, "data\n").unwrap();
        let json_file = dir.join("old.json");
        fs::write(&json_file, "data\n").unwrap();

        let logger = SessionLogger::new(&dir, 0).unwrap(); // 0-day retention
        logger.rotate();

        // Non-jsonl files should never be deleted regardless of age
        assert!(txt_file.exists());
        assert!(json_file.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
