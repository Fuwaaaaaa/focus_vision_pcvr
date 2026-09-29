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

/// SteamVR's install directory (`…\steamapps\common\SteamVR`), in whichever
/// Steam library it is:
///   1. `runtime` in `%LOCALAPPDATA%\openvr\openvrpaths.vrpath`, which
///      SteamVR writes when it first starts
///   2. every library in `libraryfolders.vdf` of the Steam install (the
///      registry's `InstallPath`, else the default locations), the Steam
///      folder itself first
///
/// Only a directory holding `bin\win64\vrpathreg.exe` counts.
/// REGRESSION: only the Steam folder itself was searched when the registry
/// had it, so SteamVR in another library (D:\SteamLibrary…) was never found.
pub fn find_steamvr_dir() -> Option<PathBuf> {
    let steam_roots: Vec<PathBuf> = read_steam_install_path_from_registry()
        .into_iter()
        .chain([PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"C:\Program Files\Steam")])
        .collect();
    steamvr_candidates(read_vrpath().as_deref(), &steam_roots).into_iter().find(|dir| is_steamvr(dir))
}

/// Where SteamVR may be, most reliable first: the vrpath's `runtime`
/// entries, then `steamapps\common\SteamVR` in each Steam root and each
/// library its `libraryfolders.vdf` lists.
fn steamvr_candidates(vrpath_json: Option<&str>, steam_roots: &[PathBuf]) -> Vec<PathBuf> {
    let from_vrpath = vrpath_json.map(|json| vrpath_list(json, "runtime")).unwrap_or_default();
    let from_libraries = steam_roots.iter().flat_map(|root| {
        let vdf = fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")).unwrap_or_default();
        std::iter::once(root.clone())
            .chain(library_paths(&vdf))
            .map(|library| library.join("steamapps").join("common").join("SteamVR"))
            .collect::<Vec<_>>()
    });
    from_vrpath.into_iter().chain(from_libraries).collect()
}

fn is_steamvr(dir: &Path) -> bool {
    vrpathreg(dir).is_file()
}

fn vrpathreg(steamvr: &Path) -> PathBuf {
    steamvr.join("bin").join("win64").join("vrpathreg.exe")
}

/// The libraries listed in a `libraryfolders.vdf`: each `"path"` value,
/// with the VDF's escaped backslashes (`D:\\SteamLibrary`) undone.
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            if quoted.next()? != "path" {
                return None;
            }
            Some(PathBuf::from(quoted.next()?.replace(r"\\", r"\")))
        })
        .collect()
}

/// Find the SteamVR driver directory (SteamVR's `drivers`), in any Steam
/// library: see [`find_steamvr_dir`].
pub fn find_steamvr_drivers_dir() -> Option<PathBuf> {
    find_steamvr_dir().map(|d| d.join("drivers")).filter(|d| d.is_dir())
}

/// How registering our driver with SteamVR failed.
#[derive(Debug)]
pub enum RegisterError {
    /// No SteamVR in any Steam library.
    SteamVrNotFound,
    /// vrpathreg couldn't run, or refused.
    Vrpathreg(String),
}

/// Register (`adddriver`) or unregister (`removedriver`) `driver_dir` with
/// SteamVR through its vrpathreg, as `vrpathreg <command> <driver_dir>`.
pub fn vrpathreg_driver(command: &str, driver_dir: &Path) -> Result<PathBuf, RegisterError> {
    let steamvr = find_steamvr_dir().ok_or(RegisterError::SteamVrNotFound)?;
    let tool = vrpathreg(&steamvr);
    let status = crate::process::command(&tool)
        .arg(command)
        .arg(driver_dir)
        .status()
        .map_err(|e| RegisterError::Vrpathreg(format!("{}: {e}", tool.display())))?;
    if status.success() {
        Ok(steamvr)
    } else {
        Err(RegisterError::Vrpathreg(format!("{} {command} exited with {status}", tool.display())))
    }
}

/// Exit codes of `--register-driver` / `--unregister-driver`, which the
/// installer reads.
pub const EXIT_OK: i32 = 0;
pub const EXIT_STEAMVR_NOT_FOUND: i32 = 2;
pub const EXIT_VRPATHREG_FAILED: i32 = 3;

/// The installer's command line: `--register-driver <dir>` or
/// `--unregister-driver <dir>` runs vrpathreg and returns the exit code;
/// `None` for any other command line (the app starts).
pub fn run_cli(args: &[String]) -> Option<i32> {
    let command = match args.first()?.as_str() {
        "--register-driver" => "adddriver",
        "--unregister-driver" => "removedriver",
        _ => return None,
    };
    let Some(dir) = args.get(1) else {
        log::error!("{} needs the driver directory", args[0]);
        return Some(EXIT_VRPATHREG_FAILED);
    };
    Some(match vrpathreg_driver(command, Path::new(dir)) {
        Ok(steamvr) => {
            log::info!("vrpathreg {command} {dir}: done (SteamVR at {})", steamvr.display());
            EXIT_OK
        }
        Err(RegisterError::SteamVrNotFound) => {
            log::error!("SteamVR not found in any Steam library");
            EXIT_STEAMVR_NOT_FOUND
        }
        Err(RegisterError::Vrpathreg(e)) => {
            log::error!("{e}");
            EXIT_VRPATHREG_FAILED
        }
    })
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
    registered_driver_in(&read_vrpath()?)
}

fn registered_driver_in(vrpath_json: &str) -> Option<PathBuf> {
    vrpath_list(vrpath_json, "external_drivers").into_iter().find(|d| has_driver_dll(d))
}

/// SteamVR's log directory (vrserver.txt, vrcompositor.txt): `log` in
/// `openvrpaths.vrpath`, else `logs` under Steam's install path.
pub fn find_steamvr_log_dir() -> Option<PathBuf> {
    read_vrpath()
        .and_then(|json| log_dir_in(&json))
        .or_else(|| read_steam_install_path_from_registry().map(|steam| steam.join("logs")).filter(|d| d.is_dir()))
}

fn log_dir_in(vrpath_json: &str) -> Option<PathBuf> {
    vrpath_list(vrpath_json, "log").into_iter().find(|d| d.is_dir())
}

/// `%LOCALAPPDATA%\openvr\openvrpaths.vrpath`, where SteamVR records its
/// runtime, config, log and external driver directories.
fn read_vrpath() -> Option<String> {
    fs::read_to_string(dirs_next::cache_dir()?.join("openvr").join("openvrpaths.vrpath")).ok()
}

/// The directories listed under `key` in an openvrpaths.vrpath.
fn vrpath_list(vrpath_json: &str, key: &str) -> Vec<PathBuf> {
    // SteamVR may write the file with a UTF-8 BOM.
    let paths: Option<serde_json::Value> = serde_json::from_str(vrpath_json.trim_start_matches('\u{feff}')).ok();
    paths
        .as_ref()
        .and_then(|p| p.get(key)?.as_array())
        .map(|dirs| dirs.iter().filter_map(|d| d.as_str()).map(PathBuf::from).collect())
        .unwrap_or_default()
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
    fn steamvr_logs_are_where_the_vrpath_says() {
        let gone = std::env::temp_dir().join("fvp-no-such-log-dir");
        let logs = TempDriverDir::new("logs", false);
        let json = serde_json::json!({
            "log": [gone.to_string_lossy(), logs.0.to_string_lossy()],
            "runtime": [r"C:\Steam\steamapps\common\SteamVR"],
            "version": 1,
        })
        .to_string();
        assert_eq!(log_dir_in(&json), Some(logs.0.clone()), "the first that exists");
        assert_eq!(log_dir_in(&format!("\u{feff}{json}")), Some(logs.0.clone()));
        assert_eq!(log_dir_in(r#"{"version": 1}"#), None);
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

    const LIBRARYFOLDERS: &str = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"contentid"		"4512345678901234567"
		"apps"
		{
			"228980"		"123456"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"label"		""
		"apps"
		{
			"250820"		"5678901234"
		}
	}
	"2"
	{
		"path"		"E:\\ゲーム\\Steam"
	}
}
"#;

    #[test]
    fn libraries_come_from_libraryfolders_vdf() {
        assert_eq!(
            library_paths(LIBRARYFOLDERS),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
                PathBuf::from(r"E:\ゲーム\Steam"),
            ]
        );
        assert!(library_paths("").is_empty());
    }

    /// A fake Steam install at `root` whose libraryfolders.vdf lists
    /// `libraries`.
    fn fake_steam(root: &Path, libraries: &[&Path]) {
        let mut vdf = String::from("\"libraryfolders\"\n{\n");
        for (i, library) in libraries.iter().enumerate() {
            let escaped = library.to_string_lossy().replace('\\', r"\\");
            vdf.push_str(&format!("\t\"{i}\"\n\t{{\n\t\t\"path\"\t\t\"{escaped}\"\n\t}}\n"));
        }
        vdf.push_str("}\n");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(root.join("steamapps").join("libraryfolders.vdf"), vdf).unwrap();
    }

    fn fake_steamvr(library: &Path) -> PathBuf {
        let steamvr = library.join("steamapps").join("common").join("SteamVR");
        fs::create_dir_all(steamvr.join("bin").join("win64")).unwrap();
        fs::write(vrpathreg(&steamvr), b"").unwrap();
        steamvr
    }

    #[test]
    fn steamvr_in_another_library_is_found() {
        // REGRESSION: with Steam in the registry, only its own folder was
        // searched, so SteamVR on D:\SteamLibrary was never found.
        let steam = TempDriverDir::new("steam-root", false);
        let library = TempDriverDir::new("steam-library", false);
        fake_steam(&steam.0, &[&steam.0, &library.0]);
        let steamvr = fake_steamvr(&library.0);
        let found = steamvr_candidates(None, std::slice::from_ref(&steam.0)).into_iter().find(|d| is_steamvr(d));
        assert_eq!(found, Some(steamvr));
    }

    #[test]
    fn the_vrpath_runtime_comes_first() {
        let steam = TempDriverDir::new("steam-root2", false);
        let elsewhere = TempDriverDir::new("runtime", false);
        fake_steam(&steam.0, &[&steam.0]);
        fake_steamvr(&steam.0);
        let runtime = fake_steamvr(&elsewhere.0);
        let json = serde_json::json!({ "runtime": [runtime.to_string_lossy()], "version": 1 }).to_string();
        let found = steamvr_candidates(Some(&json), std::slice::from_ref(&steam.0)).into_iter().find(|d| is_steamvr(d));
        assert_eq!(found, Some(runtime));
    }

    #[test]
    fn a_folder_without_vrpathreg_is_not_steamvr() {
        let dir = TempDriverDir::new("not-steamvr", false);
        assert!(!is_steamvr(&dir.0));
    }

    #[test]
    fn only_the_installers_arguments_run_the_cli() {
        assert_eq!(run_cli(&[]), None);
        assert_eq!(run_cli(&["--demo".to_string()]), None, "the app starts");
        assert_eq!(run_cli(&["--register-driver".to_string()]), Some(EXIT_VRPATHREG_FAILED), "no directory");
    }
}
