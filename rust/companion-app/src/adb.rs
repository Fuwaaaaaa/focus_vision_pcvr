use std::process::Output;
use std::time::Duration;

use crate::process;

/// How long an adb command may take before it is given up on. adb waits
/// forever on a device that stopped answering.
/// REGRESSION: nothing timed out, so a stuck install left Deploy on
/// "Installing..." for good, and a stuck device scan froze the window.
pub const QUICK: Duration = Duration::from_secs(10);
/// An install copies the APK over USB and verifies it.
pub const INSTALL: Duration = Duration::from_secs(180);
/// A logcat dump for the diagnostics zip.
pub const LOGCAT: Duration = Duration::from_secs(30);

/// Run `adb <args>`, killed after `timeout`. The error says why.
pub fn run(adb_path: &str, args: &[&str], timeout: Duration) -> Result<Output, String> {
    process::output_with_timeout(process::command(adb_path).args(args), timeout).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            format!("adb got {e} — is the headset awake, still connected, and USB debugging allowed on it?")
        } else {
            format!("Failed to run adb: {e}")
        }
    })
}

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
    if run("adb", &["version"], QUICK).is_ok() {
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
        let field = |key: &str| parts.iter().find_map(|p| p.strip_prefix(key)).map(str::to_string);
        let model = field("model:").unwrap_or_else(|| "Unknown".to_string());

        // A VIVE headset by its model, product or device name (the Focus
        // Vision's exact strings are not yet seen on the hardware).
        let is_focus_vision = [field("model:"), field("product:"), field("device:")]
            .iter()
            .flatten()
            .any(|name| {
                let name = name.to_lowercase();
                name.contains("vive") || name.contains("focus")
            });

        devices.push(AdbDevice { serial, model, is_focus_vision });
    }

    devices
}

/// List connected ADB devices. Blocks up to `QUICK`: call it off the UI
/// thread.
pub fn list_devices(adb_path: &str) -> Vec<AdbDevice> {
    match run(adb_path, &["devices", "-l"], QUICK) {
        Ok(output) => parse_device_list(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => vec![],
    }
}

/// The devices Deploy installs on: the VIVE headsets. REGRESSION: it
/// installed on (and started the app on) every device adb listed — a phone
/// plugged in to charge, another headset.
pub fn deploy_targets(devices: &[AdbDevice]) -> Vec<String> {
    devices.iter().filter(|d| d.is_focus_vision).map(|d| d.serial.clone()).collect()
}

/// Install APK on a device via ADB.
/// Returns Ok(output) on success, Err(error) on failure.
pub fn install_apk(adb_path: &str, serial: &str, apk_path: &str) -> Result<String, String> {
    let output = run(adb_path, &["-s", serial, "install", "-r", apk_path], INSTALL)?;

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
    let output = run(adb_path, &["-s", serial, "logcat", "-d", "-s", "FocusVision:*"], LOGCAT)?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Launch the app on the device.
pub fn launch_app(adb_path: &str, serial: &str, package: &str) -> Result<String, String> {
    let activity = format!("{package}/.MainActivity");
    let output = run(adb_path, &["-s", serial, "shell", "am", "start", "-n", &activity], QUICK)?;

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
    fn deploy_goes_to_the_vive_headsets_only() {
        let output = "\
List of devices attached
SERIAL001   device usb:1-1 product:vive model:VIVE_Focus_Vision transport_id:1
SERIAL002   device usb:1-2 product:quest model:Quest_3 transport_id:2
SERIAL003   device usb:1-3 product:pixel model:Pixel_7 transport_id:3
";
        assert_eq!(deploy_targets(&parse_device_list(output)), ["SERIAL001"]);
        assert!(deploy_targets(&parse_device_list("List of devices attached\nS  device model:Pixel_7\n")).is_empty());
    }

    #[test]
    fn a_vive_headset_is_known_by_its_product_or_device_name_too() {
        let output = "\
List of devices attached
SERIAL005   device usb:1-5 product:vive_focus_vision model:HMD device:focusvision transport_id:5
";
        assert!(parse_device_list(output)[0].is_focus_vision);
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
