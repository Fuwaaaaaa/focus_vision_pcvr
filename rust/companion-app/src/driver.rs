use std::path::{Path, PathBuf};
use std::fs;

/// Read Steam's `InstallPath` from the Windows registry. Steam writes this
/// value at install time, and it survives drive-letter changes, custom
/// install locations, and the user moving Steam to a different folder
/// (the registry value is rewritten on the next Steam launch).
///
/// Two lookup keys, in order:
///   1. `HKLM\Software\WOW6432Node\Valve\Steam\InstallPath` — Steam is a
///      32-bit app, so on 64-bit Windows (the only host we ship on) the
///      installer writes here.
///   2. `HKLM\Software\Valve\Steam\InstallPath` — defensive fallback for
///      rare 32-bit Windows hosts or a future 64-bit Steam build.
///
/// Returns the absolute path to the Steam install dir (without the
/// `steamapps\common\SteamVR\drivers` suffix), or `None` on failure.
#[cfg(target_os = "windows")]
fn read_steam_install_path_from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for subkey in &["SOFTWARE\\WOW6432Node\\Valve\\Steam", "SOFTWARE\\Valve\\Steam"] {
        if let Ok(key) = hklm.open_subkey(subkey) {
            if let Ok(install_path) = key.get_value::<String, _>("InstallPath") {
                if !install_path.is_empty() {
                    return Some(PathBuf::from(install_path));
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn read_steam_install_path_from_registry() -> Option<PathBuf> {
    None
}

/// Find the SteamVR driver directory.
/// Lookup order (most reliable first):
///   1. Windows registry — `HKLM\...\Valve\Steam\InstallPath`
///   2. Hard-coded common paths (Program Files x86/x64, D: drive)
///   3. libraryfolders.vdf parsing for users with Steam libraries on
///      other drives configured via Steam's UI
pub fn find_steamvr_drivers_dir() -> Option<PathBuf> {
    // 1. Registry lookup — works for custom install locations on any drive.
    if let Some(steam_root) = read_steam_install_path_from_registry() {
        let driver_dir = steam_root
            .join("steamapps").join("common").join("SteamVR").join("drivers");
        if driver_dir.exists() {
            return Some(driver_dir);
        }
        // The registry InstallPath was real but SteamVR isn't installed.
        // Don't fall through to the hard-coded list — that would just race
        // back to a different Steam install. Return None so the UI can
        // prompt "install SteamVR first" instead of silently picking the
        // wrong copy.
        return None;
    }

    // 2. Hard-coded common paths — for the (rare) case where Steam's
    //    registry entry is missing or unreadable but the install dir is
    //    still in the usual place.
    let candidates = [
        "C:\\Program Files (x86)\\Steam\\steamapps\\common\\SteamVR\\drivers",
        "C:\\Program Files\\Steam\\steamapps\\common\\SteamVR\\drivers",
        "D:\\Steam\\steamapps\\common\\SteamVR\\drivers",
        "D:\\SteamLibrary\\steamapps\\common\\SteamVR\\drivers",
    ];

    for path in &candidates {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    // 3. libraryfolders.vdf — for Steam-managed libraries on non-default
    //    drives. We only consult this if neither the registry nor the
    //    common paths panned out, since they would be cheaper.
    let vdf_paths = [
        "C:\\Program Files (x86)\\Steam\\steamapps\\libraryfolders.vdf",
        "C:\\Program Files\\Steam\\steamapps\\libraryfolders.vdf",
    ];

    for vdf_path in &vdf_paths {
        if let Ok(content) = fs::read_to_string(vdf_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("\"path\"") {
                    if let Some(path) = line.split('"').nth(3) {
                        let driver_path = PathBuf::from(path)
                            .join("steamapps")
                            .join("common")
                            .join("SteamVR")
                            .join("drivers");
                        if driver_path.exists() {
                            return Some(driver_path);
                        }
                    }
                }
            }
        }
    }

    None
}

const DRIVER_DLL: &str = "driver_focus_vision_pcvr.dll";

/// Whether `driver_dir` holds our driver the way SteamVR loads it.
fn has_driver_dll(driver_dir: &Path) -> bool {
    driver_dir.join("bin").join("win64").join(DRIVER_DLL).exists()
}

/// Check if our driver is already installed.
pub fn is_driver_installed(drivers_dir: &Path) -> bool {
    has_driver_dll(&drivers_dir.join("focus_vision_pcvr"))
}

/// The directory of our driver if it is registered with SteamVR through
/// `vrpathreg adddriver` — what the installer does, instead of copying it
/// into SteamVR's `drivers` folder. SteamVR lists those directories under
/// `external_drivers` in `%LOCALAPPDATA%\openvr\openvrpaths.vrpath`.
pub fn find_registered_driver() -> Option<PathBuf> {
    let path = dirs_next::cache_dir()?.join("openvr").join("openvrpaths.vrpath");
    registered_driver_in(&fs::read_to_string(path).ok()?)
}

fn registered_driver_in(vrpath_json: &str) -> Option<PathBuf> {
    let paths: serde_json::Value =
        serde_json::from_str(vrpath_json.trim_start_matches('\u{feff}')).ok()?;
    paths.get("external_drivers")?
        .as_array()?
        .iter()
        .filter_map(|d| d.as_str())
        .map(PathBuf::from)
        .find(|d| has_driver_dll(d))
}

/// Install our driver into SteamVR's drivers directory.
/// `driver_source`: our built driver directory (`driver.vrdrivermanifest`,
/// `bin/win64/`, `resources/`).
pub fn install_driver(drivers_dir: &Path, driver_source: &Path) -> Result<(), String> {
    let target = drivers_dir.join("focus_vision_pcvr");

    // Create directory structure
    fs::create_dir_all(target.join("bin").join("win64"))
        .map_err(|e| format!("Failed to create driver directory: {e}"))?;

    // Copy DLL
    let dll_name = DRIVER_DLL;
    let src_dll = driver_source.join("bin").join("win64").join(dll_name);
    if !src_dll.exists() {
        return Err(format!("Driver DLL not found: {}", src_dll.display()));
    }
    fs::copy(&src_dll, target.join("bin").join("win64").join(dll_name))
        .map_err(|e| format!("Failed to copy DLL: {e}"))?;

    // Copy manifest
    let manifest = "driver.vrdrivermanifest";
    let src_manifest = driver_source.join(manifest);
    if src_manifest.exists() {
        fs::copy(&src_manifest, target.join(manifest))
            .map_err(|e| format!("Failed to copy manifest: {e}"))?;
    }

    // Copy resources directory
    let src_resources = driver_source.join("resources");
    if src_resources.exists() {
        copy_dir_recursive(&src_resources, &target.join("resources"))?;
    }

    Ok(())
}

/// Uninstall our driver from SteamVR.
pub fn uninstall_driver(drivers_dir: &Path) -> Result<(), String> {
    let target = drivers_dir.join("focus_vision_pcvr");
    if target.exists() {
        fs::remove_dir_all(&target)
            .map_err(|e| format!("Failed to remove driver: {e}"))?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("mkdir failed: {e}"))?;
    for entry in fs::read_dir(src).map_err(|e| format!("readdir failed: {e}"))? {
        let entry = entry.map_err(|e| format!("entry error: {e}"))?;
        let ty = entry.file_type().map_err(|e| format!("filetype error: {e}"))?;
        let dest = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest)
                .map_err(|e| format!("copy failed: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch driver directory, removed on drop.
    struct TempDriverDir(PathBuf);
    impl TempDriverDir {
        fn new(name: &str, with_dll: bool) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("fv-driver-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("bin").join("win64")).unwrap();
            if with_dll {
                fs::write(dir.join("bin").join("win64").join(DRIVER_DLL), b"").unwrap();
            }
            Self(dir)
        }
    }
    impl Drop for TempDriverDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn vrpath(dirs: &[&Path]) -> String {
        serde_json::json!({
            "config": [r"C:\Steam\config"],
            "external_drivers": dirs.iter().map(|d| d.to_string_lossy()).collect::<Vec<_>>(),
            "jsonid": "vrpathreg",
            "version": 1,
        })
        .to_string()
    }

    #[test]
    fn finds_the_registered_directory_that_holds_our_dll() {
        let other = TempDriverDir::new("other", false);
        let ours = TempDriverDir::new("ours", true);
        let json = vrpath(&[&other.0, &ours.0]);
        assert_eq!(registered_driver_in(&json), Some(ours.0.clone()));
        // SteamVR may write the file with a UTF-8 BOM.
        assert_eq!(registered_driver_in(&format!("\u{feff}{json}")), Some(ours.0.clone()));
    }

    #[test]
    fn a_registration_without_the_dll_does_not_count() {
        let stale = TempDriverDir::new("stale", false);
        assert_eq!(registered_driver_in(&vrpath(&[&stale.0])), None);
    }

    #[test]
    fn missing_or_malformed_external_drivers_is_none() {
        assert_eq!(registered_driver_in(r#"{"external_drivers": null, "version": 1}"#), None);
        assert_eq!(registered_driver_in(r#"{"version": 1}"#), None);
        assert_eq!(registered_driver_in("not json"), None);
    }

    #[test]
    fn install_copies_the_dll_from_the_build_layout() {
        let source = TempDriverDir::new("source", true);
        fs::write(source.0.join("driver.vrdrivermanifest"), b"{}").unwrap();
        let steamvr = TempDriverDir::new("steamvr-drivers", false);
        install_driver(&steamvr.0, &source.0).unwrap();
        assert!(is_driver_installed(&steamvr.0));
        assert!(steamvr.0.join("focus_vision_pcvr").join("driver.vrdrivermanifest").exists());
    }
}
