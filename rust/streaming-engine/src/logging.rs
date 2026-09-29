//! The engine's log, kept in a file. Under SteamVR the engine runs inside
//! vrserver.exe, whose stderr goes nowhere, so `env_logger`'s output was
//! lost — and without `RUST_LOG` it kept errors only.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The log, next to status.json; the companion's "Export Logs" collects it.
const LOG_FILE: &str = "engine.log";
/// The one before: the previous run's, or the first 16 MB of a long run.
const PREVIOUS_LOG_FILE: &str = "engine.prev.log";
/// Size at which the log starts over.
const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;

/// A log file that starts over when it reaches `max_bytes`, and at open,
/// moving what it had to `engine.prev.log`.
struct RotatingLog {
    path: PathBuf,
    previous: PathBuf,
    file: File,
    written: u64,
    max_bytes: u64,
}

impl RotatingLog {
    fn open(dir: &Path, max_bytes: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(LOG_FILE);
        let previous = dir.join(PREVIOUS_LOG_FILE);
        let file = Self::start(&path, &previous)?;
        Ok(Self { path, previous, file, written: 0, max_bytes })
    }

    /// Move `path` over `previous` and create it anew. If it can't be moved
    /// (another process holds it), it is emptied instead.
    fn start(path: &Path, previous: &Path) -> io::Result<File> {
        if path.exists() {
            let _ = fs::rename(path, previous);
        }
        OpenOptions::new().create(true).write(true).truncate(true).open(path)
    }
}

impl Write for RotatingLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // env_logger writes a record at a time, so records aren't split.
        if self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            self.file = Self::start(&self.path, &self.previous)?;
            self.written = 0;
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Log to `dir`/engine.log: the engine at info, other crates at warn
/// (`RUST_LOG` overrides), with millisecond UTC timestamps. To stderr if
/// `dir` is `None` or the file can't be opened. The first call in a process
/// wins; later ones do nothing.
pub fn init(dir: Option<&Path>) {
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,streaming_engine=info,fvp_common=info"),
    );
    builder.format_timestamp_millis();
    let file = dir.map(|d| RotatingLog::open(d, MAX_LOG_BYTES));
    let mut failure = None;
    match file {
        Some(Ok(log)) => {
            builder.target(env_logger::Target::Pipe(Box::new(log)));
        }
        Some(Err(e)) => failure = Some(e),
        None => {}
    }
    if builder.try_init().is_ok() {
        if let Some(e) = failure {
            log::warn!("Engine log file unavailable ({e}); logging to stderr");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fvp-logging-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_new_run_keeps_the_previous_runs_log() {
        let dir = temp_dir("runs");
        {
            let mut log = RotatingLog::open(&dir, 1024).unwrap();
            log.write_all(b"first run\n").unwrap();
        }
        let mut log = RotatingLog::open(&dir, 1024).unwrap();
        log.write_all(b"second run\n").unwrap();
        log.flush().unwrap();
        assert_eq!(fs::read_to_string(dir.join(LOG_FILE)).unwrap(), "second run\n");
        assert_eq!(fs::read_to_string(dir.join(PREVIOUS_LOG_FILE)).unwrap(), "first run\n");
        drop(log);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_log_starts_over_between_records() {
        let dir = temp_dir("full");
        let mut log = RotatingLog::open(&dir, 32).unwrap();
        log.write_all(b"record one: 20 bytes").unwrap();
        log.write_all(b"record two: 20 bytes").unwrap(); // would pass 32
        log.flush().unwrap();
        assert_eq!(fs::read_to_string(dir.join(LOG_FILE)).unwrap(), "record two: 20 bytes");
        assert_eq!(fs::read_to_string(dir.join(PREVIOUS_LOG_FILE)).unwrap(), "record one: 20 bytes");
        drop(log);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_record_longer_than_the_limit_is_still_written() {
        let dir = temp_dir("long");
        let mut log = RotatingLog::open(&dir, 8).unwrap();
        log.write_all(b"longer than eight bytes").unwrap();
        log.flush().unwrap();
        assert_eq!(fs::read_to_string(dir.join(LOG_FILE)).unwrap(), "longer than eight bytes");
        drop(log);
        let _ = fs::remove_dir_all(&dir);
    }
}
