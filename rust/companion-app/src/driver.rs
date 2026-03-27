use std::path::{Path, PathBuf};
use std::fs;

/// Find the SteamVR driver directory.
/// Checks common Steam install paths and reads libraryfolders.vdf.
pub fn find_steamvr_drivers_dir() -> Option<PathBuf> {
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

    // Try reading Steam's libraryfolders.vdf for custom paths
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
