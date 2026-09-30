//! Child processes (adb, PowerShell, wmic) started without a console window.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// `Command::new(program)` that opens no console window. The release exe is
/// a GUI app without a console, so Windows would give every console program
/// it starts a window of its own — one flashing up at each adb device scan.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)] // only changed on Windows
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `cmd.output()`, but a program still running after `timeout` is killed
/// and the result is a `TimedOut` error. adb waits forever on a device
/// that stopped answering (asleep, unplugged mid-install, a USB prompt
/// nobody accepted); without this the caller waited with it.
pub(crate) fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stdout = Drain::start(child.stdout.take());
    let stderr = Drain::start(child.stderr.take());

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("no answer within {} s", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok(Output { status, stdout: stdout.finish(), stderr: stderr.finish() })
}

/// Reads a child's pipe as it writes (a full pipe would stall it), keeping
/// what came so far.
struct Drain {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drain {
    fn start(pipe: Option<impl Read + Send + 'static>) -> Self {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = bytes.clone();
        let thread = pipe.map(|mut pipe| {
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                    if let Ok(mut all) = sink.lock() {
                        all.extend_from_slice(&chunk[..n]);
                    }
                }
            })
        });
        Self { bytes, thread }
    }

    /// What the program wrote. It has exited, but a process it started (adb
    /// starting its server) can hold the pipe open, so the end of the pipe
    /// is waited for only briefly.
    fn finish(mut self) -> Vec<u8> {
        let until = Instant::now() + Duration::from_millis(500);
        if let Some(thread) = self.thread.take() {
            while !thread.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.bytes.lock().map(|mut all| std::mem::take(&mut *all)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn hidden_command_still_runs_and_captures_output() {
        let out = super::command("cmd").args(["/C", "echo fvp"]).output().expect("cmd runs");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "fvp");
    }

    #[cfg(windows)]
    #[test]
    fn a_program_that_finishes_in_time_gives_its_output() {
        use std::time::Duration;
        let out = super::output_with_timeout(super::command("cmd").args(["/C", "echo fvp& echo err 1>&2"]), Duration::from_secs(10))
            .expect("finishes");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "fvp");
        assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "err");
    }

    #[cfg(windows)]
    #[test]
    fn a_program_that_hangs_is_killed_at_the_timeout() {
        use std::time::{Duration, Instant};
        // ping -n 30 takes ~29 s.
        let started = Instant::now();
        let err = super::output_with_timeout(super::command("ping").args(["-n", "30", "127.0.0.1"]), Duration::from_millis(500))
            .expect_err("times out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5), "returned after {:?}", started.elapsed());
    }
}
