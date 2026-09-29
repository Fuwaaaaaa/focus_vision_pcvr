//! Open / save dialogs, through PowerShell's Windows Forms (no dialog crate),
//! each on its own thread: a dialog blocks until it is closed, and on the UI
//! thread it froze the window meanwhile.

#[cfg(target_os = "windows")]
use crate::process;
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc;

/// The end of each dialog script: the chosen path as hex of its UTF-8
/// bytes. PowerShell 5.1 writes stdout in the console's OEM code page (932
/// on Japanese Windows), which mangled every non-ASCII path.
#[cfg(target_os = "windows")]
const PRINT_PATH: &str = "[BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($path)).Replace('-', '')";

/// The path a dialog script printed (`PRINT_PATH`). `None` when it printed
/// nothing (cancelled) or anything else.
fn decode_path(stdout: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(stdout).ok()?.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let (pairs, rest) = line.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let digit = |c: u8| char::from(c).to_digit(16);
    let bytes = pairs
        .iter()
        .map(|[hi, lo]| Some((digit(*hi)? * 16 + digit(*lo)?) as u8))
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// Run a dialog script (Windows Forms needs an STA thread; `-NoProfile` so
/// a profile's output can't mix into the path) with `env` set.
#[cfg(target_os = "windows")]
fn run_dialog(script: &str, env: &[(&str, &str)]) -> Option<String> {
    let mut command = process::command("powershell");
    for (key, value) in env {
        command.env(key, value);
    }
    let script = format!("Add-Type -AssemblyName System.Windows.Forms\n{script}\nif ($path) {{ {PRINT_PATH} }}");
    let output = command.args(["-NoProfile", "-STA", "-Command", &script]).output().ok()?;
    decode_path(&output.stdout)
}

/// Ask for an APK to install. Blocks until the dialog closes.
pub(crate) fn pick_apk() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        run_dialog(
            r#"
            $dialog = New-Object System.Windows.Forms.OpenFileDialog
            $dialog.Filter = 'APK files (*.apk)|*.apk|All files (*.*)|*.*'
            $dialog.Title = 'Select APK to install'
            $path = if ($dialog.ShowDialog() -eq 'OK') { $dialog.FileName }
            "#,
            &[],
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Ask where to save a file: `default_stem` is the suggested name, `ext`
/// the extension without a dot. Blocks until the dialog closes; `None` if
/// cancelled or no dialog is available.
pub(crate) fn pick_save_path(default_stem: &str, ext: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        // Values go in through env vars rather than the script body, so
        // quotes or backticks in them can't break the PowerShell quoting.
        run_dialog(
            r#"
            $stem = $env:FVP_SAVE_STEM
            $ext = $env:FVP_SAVE_EXT
            $dialog = New-Object System.Windows.Forms.SaveFileDialog
            $dialog.Filter = "$ext files (*.$ext)|*.$ext|All files (*.*)|*.*"
            $dialog.Title = 'Save'
            $dialog.FileName = "$stem.$ext"
            $dialog.DefaultExt = $ext
            $dialog.AddExtension = $true
            $path = if ($dialog.ShowDialog() -eq 'OK') { $dialog.FileName }
            "#,
            &[("FVP_SAVE_STEM", default_stem), ("FVP_SAVE_EXT", ext)],
        )
        .map(PathBuf::from)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (default_stem, ext);
        None
    }
}

/// A dialog open on its own thread; the UI keeps drawing and asks each
/// frame whether it has closed.
pub(crate) struct DialogTask<T> {
    answer: mpsc::Receiver<Option<T>>,
}

impl<T: Send + 'static> DialogTask<T> {
    /// Run `dialog` on a new thread, repainting `ctx` when it closes.
    pub(crate) fn spawn(ctx: &egui::Context, dialog: impl FnOnce() -> Option<T> + Send + 'static) -> Self {
        let (tx, answer) = mpsc::channel();
        let ctx = ctx.clone();
        // Without a thread the receiver is disconnected: read as cancelled.
        let _ = std::thread::Builder::new().name("fvp-file-dialog".into()).spawn(move || {
            let _ = tx.send(dialog());
            ctx.request_repaint();
        });
        Self { answer }
    }

    /// `Some(answer)` once the dialog has closed (`Some(None)`: cancelled),
    /// `None` while it is open.
    pub(crate) fn poll(&self) -> Option<Option<T>> {
        match self.answer.try_recv() {
            Ok(answer) => Some(answer),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02X}")).collect()
    }

    #[test]
    fn paths_come_back_whatever_their_characters() {
        // REGRESSION: stdout was read as UTF-8 but written in code page
        // 932, so a path under a Japanese user name came back mangled.
        for path in [r"C:\Users\山田\Downloads\focus-vision.apk", r"D:\ゲーム\stats.svg", r"C:\plain\a.apk"] {
            assert_eq!(decode_path(format!("{}\r\n", hex(path)).as_bytes()).as_deref(), Some(path));
        }
        assert_eq!(decode_path(b"433a5c612e61706b").as_deref(), Some(r"C:\a.apk"), "lower case hex");
    }

    #[test]
    fn anything_but_a_hex_path_is_no_path() {
        assert_eq!(decode_path(b""), None, "cancelled");
        assert_eq!(decode_path(b"\r\n"), None);
        assert_eq!(decode_path(b"433"), None, "odd length");
        assert_eq!(decode_path(b"C:\\a.apk"), None, "not hex");
        assert_eq!(decode_path(b"FF"), None, "not UTF-8");
        assert_eq!(decode_path("山田".as_bytes()), None);
    }

    #[test]
    fn a_warning_before_the_path_is_skipped() {
        let out = format!("WARNING: something\r\n{}\r\n", hex(r"C:\a.apk"));
        assert_eq!(decode_path(out.as_bytes()).as_deref(), Some(r"C:\a.apk"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_prints_a_japanese_path_intact() {
        // The real round trip, without the dialog: PowerShell 5.1 with the
        // system's code page, the path in through an env var.
        let path = r"C:\Users\山田\ダウンロード\focus-vision.apk";
        let script = format!("$path = $env:FVP_TEST_PATH\n{PRINT_PATH}");
        let out = process::command("powershell")
            .env("FVP_TEST_PATH", path)
            .args(["-NoProfile", "-Command", &script])
            .output()
            .expect("powershell runs");
        assert_eq!(decode_path(&out.stdout).as_deref(), Some(path));
    }

    #[test]
    fn a_dialog_task_answers_once_its_thread_is_done() {
        let ctx = egui::Context::default();
        let (release, gate) = mpsc::channel::<()>();
        let task = DialogTask::spawn(&ctx, move || {
            gate.recv().ok()?;
            Some("chosen".to_string())
        });
        assert_eq!(task.poll(), None, "still open");
        release.send(()).unwrap();
        let answer = loop {
            if let Some(answer) = task.poll() {
                break answer;
            }
            std::thread::yield_now();
        };
        assert_eq!(answer.as_deref(), Some("chosen"));
    }
}
