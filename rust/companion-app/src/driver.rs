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

/// Check if our driver is already installed.
pub fn is_driver_installed(drivers_dir: &Path) -> bool {
    let our_dir = drivers_dir.join("focus_vision_pcvr");
    our_dir.exists() && our_dir.join("bin").join("win64").join("driver_focus_vision_pcvr.dll").exists()
}

/// Install our driver into SteamVR's drivers directory.
/// `driver_source`: directory containing our built driver files.
pub fn install_driver(drivers_dir: &Path, driver_source: &Path) -> Result<(), String> {
    let target = drivers_dir.join("focus_vision_pcvr");

    // Create directory structure
    fs::create_dir_all(target.join("bin").join("win64"))
        .map_err(|e| format!("Failed to create driver directory: {e}"))?;

    // Copy DLL
    let dll_name = "driver_focus_vision_pcvr.dll";
    let src_dll = driver_source.join(dll_name);
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
