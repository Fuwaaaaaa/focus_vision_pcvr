use std::process::Command;

/// ADB device info
#[derive(Debug, Clone)]
pub struct AdbDevice {
    pub serial: String,
    pub model: String,
    pub is_focus_vision: bool,
}

/// Find adb.exe — check PATH, then common install locations.
pub fn find_adb() -> Option<String> {
    // Check PATH first
    if Command::new("adb").arg("version").output().is_ok() {
        return Some("adb".to_string());
    }

    // Common Android SDK locations on Windows
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let candidates = [
        format!("{home}\\AppData\\Local\\Android\\Sdk\\platform-tools\\adb.exe"),
        "C:\\Android\\platform-tools\\adb.exe".to_string(),
        "C:\\Program Files\\Android\\platform-tools\\adb.exe".to_string(),
    ];

    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.clone());
        }
    }

    None
}

/// Parse raw `adb devices -l` output into a list of devices.
pub fn parse_device_list(output: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();

    for line in output.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 || parts[1] != "device" {
            continue;
        }

        let serial = parts[0].to_string();
        let model = parts.iter()
            .find(|p| p.starts_with("model:"))
            .map(|p| p.trim_start_matches("model:").to_string())
            .unwrap_or_else(|| "Unknown".to_string());

        let is_focus_vision = model.to_lowercase().contains("focus")
            || model.to_lowercase().contains("vive");

        devices.push(AdbDevice { serial, model, is_focus_vision });
    }

    devices
}

/// List connected ADB devices.
pub fn list_devices(adb_path: &str) -> Vec<AdbDevice> {
    let output = match Command::new(adb_path).arg("devices").arg("-l").output() {
        Ok(o) => o,
        Err(_) => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_device_list(&stdout)
}

/// Install APK on a device via ADB.
/// Returns Ok(output) on success, Err(error) on failure.
pub fn install_apk(adb_path: &str, serial: &str, apk_path: &str) -> Result<String, String> {
    let output = Command::new(adb_path)
        .args(["-s", serial, "install", "-r", apk_path])
        .output()
        .map_err(|e| format!("Failed to run adb: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() && stdout.contains("Success") {
        Ok(stdout)
    } else {
        Err(format!("{stdout}\n{stderr}"))
    }
}

/// Dump logcat from the device (non-blocking — returns buffered log).
pub fn dump_logcat(adb_path: &str, serial: &str) -> Result<String, String> {
    let output = Command::new(adb_path)
        .args(["-s", serial, "logcat", "-d", "-s", "FocusVision:*"])
        .output()
        .map_err(|e| format!("Failed to run adb: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Launch the app on the device.
pub fn launch_app(adb_path: &str, serial: &str, package: &str) -> Result<String, String> {
    let activity = format!("{package}/.MainActivity");
    let output = Command::new(adb_path)
        .args(["-s", serial, "shell", "am", "start", "-n", &activity])
        .output()
        .map_err(|e| format!("Failed to run adb: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_normal_device_output() {
        let output = "\
List of devices attached
ABCDEF123456   device usb:1-1 product:vive_focus model:VIVE_Focus_Vision transport_id:1
";
        let devices = parse_device_list(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "ABCDEF123456");
        assert_eq!(devices[0].model, "VIVE_Focus_Vision");
        assert!(devices[0].is_focus_vision);
    }

    #[test]
    fn parse_empty_output() {
        let output = "\
List of devices attached

";
        let devices = parse_device_list(output);
        assert!(devices.is_empty());
    }

    #[test]
    fn parse_unauthorized_device_excluded() {
        let output = "\
List of devices attached
ABCDEF123456   unauthorized usb:1-1 transport_id:1
";
        let devices = parse_device_list(output);
        assert!(devices.is_empty());
    }

    #[test]
    fn vive_focus_model_is_focus_vision() {
        let output = "\
List of devices attached
SERIAL001   device usb:1-1 product:vive model:VIVE_Focus_3 transport_id:1
";
        let devices = parse_device_list(output);
        assert_eq!(devices.len(), 1);
        assert!(devices[0].is_focus_vision);
    }

    #[test]
    fn quest_model_not_focus_vision() {
        let output = "\
List of devices attached
SERIAL002   device usb:1-2 product:quest model:Quest_3 transport_id:2
";
        let devices = parse_device_list(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].model, "Quest_3");
        assert!(!devices[0].is_focus_vision);
    }

    #[test]
    fn parse_multiple_devices() {
        let output = "\
List of devices attached
SERIAL001   device usb:1-1 product:vive model:VIVE_Focus_Vision transport_id:1
SERIAL002   device usb:1-2 product:quest model:Quest_3 transport_id:2
SERIAL003   device usb:1-3 product:pixel model:Pixel_7 transport_id:3
";
        let devices = parse_device_list(output);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "SERIAL001");
        assert_eq!(devices[1].serial, "SERIAL002");
        assert_eq!(devices[2].serial, "SERIAL003");
    }

    #[test]
    fn parse_no_model_info_defaults_to_unknown() {
        let output = "\
List of devices attached
SERIAL004   device usb:1-4 transport_id:4
";
        let devices = parse_device_list(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].model, "Unknown");
        assert!(!devices[0].is_focus_vision);
    }
}
