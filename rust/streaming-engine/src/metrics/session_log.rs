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
    /// The PC side's average latency (frame ready → encoded → sent).
    pub pc_latency_us: u32,
    /// Frames the engine sent in the last second.
    pub pc_fps: u16,
    pub bitrate_mbps: u32,
    /// From the newest HEARTBEAT; the HMD's figures are 0 until one arrives.
    pub loss_pct: f32,
    pub fec_pct: f32,
    pub hmd_fps: u16,
    pub hmd_decode_us: u32,
    pub sleeping: bool,
    /// vrserver.exe's memory (the engine runs inside it).
    pub rss_mb: u64,
}

impl SessionRecord {
    /// The time now in the `ts` format (UTC, whole seconds).
    pub fn now_ts() -> String {
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
}

/// Session logger that buffers records and flushes to disk periodically.
///
/// - One file per session, `session_<UTC start>.jsonl` in the given directory
/// - Records are buffered in memory (60 seconds worth)
/// - Flushed to the file when the buffer interval has elapsed or on drop
/// - Old files are deleted by [`purge_old_logs`] when the engine starts
pub struct SessionLogger {
    file_path: PathBuf,
    buffer: Vec<String>,
    last_flush: Instant,
    flush_interval: Duration,
}

impl SessionLogger {
    /// Create a new session logger. Creates the directory if it doesn't exist.
    pub fn new(dir: &Path) -> Result<Self, std::io::Error> {
        fs::create_dir_all(dir)?;

        let now = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let file_name = format!("session_{}.jsonl", now);
        let file_path = dir.join(file_name);

        Ok(Self {
            file_path,
            buffer: Vec::with_capacity(6), // ~60s at 10s intervals
            last_flush: Instant::now(),
            flush_interval: Duration::from_secs(60),
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

/// Delete the `.jsonl` files in `dir` last written more than
/// `retention_days` ago. Returns how many were deleted.
pub fn purge_old_logs(dir: &Path, retention_days: u32) -> u32 {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(retention_days) * 86400))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let Ok(entries) = fs::read_dir(dir) else {
        return 0; // no sessions logged yet
    };

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if modified < cutoff {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(e) => log::warn!("Session log purge: can't delete {:?}: {}", path, e),
            }
        }
    }
    if removed > 0 {
        log::info!("Session log purge: removed {} file(s) older than {} days", removed, retention_days);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_record() -> SessionRecord {
        SessionRecord {
            ts: "2026-04-09T12:00:00Z".into(),
            pc_latency_us: 3500,
            pc_fps: 90,
            bitrate_mbps: 80,
            loss_pct: 0.5,
            fec_pct: 15.0,
            hmd_fps: 90,
            hmd_decode_us: 4000,
            sleeping: false,
            rss_mb: 420,
        }
    }

    #[test]
    fn test_session_logger_creates_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions");
        let logger = SessionLogger::new(&dir);
        assert!(logger.is_ok());
        assert!(dir.exists());
    }

    #[test]
    fn test_session_logger_write_and_flush() {
        let tmp = tempfile::tempdir().unwrap();
        let mut logger = SessionLogger::new(tmp.path()).unwrap();

        logger.record(make_record());
        logger.record(make_record());
        logger.flush();

        let content = fs::read_to_string(logger.file_path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Verify JSON structure
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed["pc_latency_us"], 3500);
        assert_eq!(parsed["hmd_fps"], 90);
        assert_eq!(parsed["sleeping"], false);
    }

    #[test]
    fn test_session_logger_drop_flushes() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path;
        {
            let mut logger = SessionLogger::new(tmp.path()).unwrap();
            logger.record(make_record());
            file_path = logger.file_path().to_path_buf();
            // Drop logger — should flush
        }
        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn test_session_logger_empty_flush_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let mut logger = SessionLogger::new(tmp.path()).unwrap();
        logger.flush(); // Should not create file
        assert!(!logger.file_path().exists());
    }

    #[test]
    fn test_ts_format() {
        let ts = SessionRecord::now_ts();
        // Should look like "2026-04-09T12:00:00Z"
        assert!(ts.contains('T'));
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        // Verify year is reasonable
        let year: u32 = ts[0..4].parse().unwrap();
        assert!((2024..=2100).contains(&year));
    }

    #[test]
    fn test_purge_removes_only_old_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let write = |name: &str, age_days: u64| {
            let path = dir.join(name);
            fs::write(&path, "data\n").unwrap();
            let mtime = SystemTime::now() - Duration::from_secs(age_days * 86400);
            fs::File::options().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
            path
        };
        let old = write("session_old.jsonl", 8);
        let recent = write("session_recent.jsonl", 6);
        let old_txt = write("notes.txt", 30);

        assert_eq!(purge_old_logs(dir, 7), 1);
        assert!(!old.exists(), "a log older than the retention is deleted");
        assert!(recent.exists(), "a recent log is kept");
        assert!(old_txt.exists(), "only .jsonl files are touched");
    }

    #[test]
    fn test_purge_without_dir_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(purge_old_logs(&tmp.path().join("missing"), 7), 0);
    }
}
