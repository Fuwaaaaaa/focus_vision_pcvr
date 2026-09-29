//! Child processes (adb, PowerShell, wmic) started without a console window.

use std::ffi::OsStr;
use std::process::Command;

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

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn hidden_command_still_runs_and_captures_output() {
        let out = super::command("cmd").args(["/C", "echo fvp"]).output().expect("cmd runs");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "fvp");
    }
}
