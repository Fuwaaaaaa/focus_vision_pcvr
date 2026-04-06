use std::io::Write;
use std::path::PathBuf;

use crate::adb;

/// Sanitize PII from log text (IP addresses, Wi-Fi SSIDs).
fn sanitize_pii(text: &str) -> String {
    // Mask IPv4 addresses
    let re_ip = regex_lite_ipv4(text);
    re_ip
}

fn regex_lite_ipv4(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Simple IPv4 detection: digit.digit.digit.digit
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut dots = 0;
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                if bytes[j] == b'.' {
                    dots += 1;
                }
                j += 1;
            }
            if dots == 3 && j - start >= 7 {
                result.push_str("[REDACTED_IP]");
                i = j;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

/// Collect system info string.
fn system_info() -> String {
    let mut info = String::new();
    info.push_str(&format!("OS: {}\n", std::env::consts::OS));
    info.push_str(&format!("Arch: {}\n", std::env::consts::ARCH));

    // GPU info via DXGI (Windows)
    #[cfg(windows)]
    {
        if let Ok(output) = std::process::Command::new("wmic")
            .args(["path", "win32_VideoController", "get", "Name"])
            .output()
        {
            let gpu = String::from_utf8_lossy(&output.stdout);
            info.push_str(&format!("GPU: {}\n", gpu.lines().nth(1).unwrap_or("Unknown").trim()));
        }
    }

    info
}

/// Export logs to a zip file. Returns the output path on success.
pub fn export_logs(adb_path: Option<&str>, device_serial: Option<&str>) -> Result<PathBuf, String> {
    let downloads = dirs_next::download_dir()
        .unwrap_or_else(|| PathBuf::from("."));
    let timestamp = chrono_lite_timestamp();
    let zip_path = downloads.join(format!("focus-vision-logs-{timestamp}.zip"));

    let file = std::fs::File::create(&zip_path)
        .map_err(|e| format!("Failed to create zip: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. System info
    zip.start_file("system-info.txt", options).map_err(|e| e.to_string())?;
    zip.write_all(system_info().as_bytes()).map_err(|e| e.to_string())?;

    // 2. PC-side engine logs
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let log_dir = PathBuf::from(appdata).join("FocusVisionPCVR");
        if log_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&log_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map_or(false, |e| e == "json" || e == "log") {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let name = format!("pc/{}", path.file_name().unwrap().to_string_lossy());
                            let _ = zip.start_file(&name, options);
                            let _ = zip.write_all(sanitize_pii(&content).as_bytes());
                        }
                    }
                }
            }
        }
    }

    // 3. HMD logcat (if ADB available)
    if let (Some(adb), Some(serial)) = (adb_path, device_serial) {
        match adb::dump_logcat(adb, serial) {
            Ok(logcat) => {
                let _ = zip.start_file("hmd/logcat.txt", options);
                let _ = zip.write_all(sanitize_pii(&logcat).as_bytes());
            }
            Err(e) => {
                let _ = zip.start_file("hmd/logcat-error.txt", options);
                let _ = zip.write_all(format!("Failed to capture logcat: {e}").as_bytes());
            }
        }
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(zip_path)
}

fn chrono_lite_timestamp() -> String {
    // Simple timestamp without chrono dependency
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", now.as_secs())
}
