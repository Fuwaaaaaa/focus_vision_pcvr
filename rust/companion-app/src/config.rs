use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// User-local config overrides (`%APPDATA%/FocusVisionPCVR/config/local.toml`).
/// Only contains fields the user explicitly changed via the companion UI.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub video: VideoOverride,
    #[serde(default)]
    pub sleep_mode: SleepModeOverride,
    #[serde(default)]
    pub face_tracking: FaceTrackingOverride,
    #[serde(default)]
    pub recording: RecordingOverride,
    #[serde(default)]
    pub audio: AudioOverride,
    #[serde(default)]
    pub deploy: DeployOverride,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VideoOverride {
    /// "h265" or "h264"
    #[serde(default = "default_codec")]
    pub codec: String,
}

impl Default for VideoOverride {
    fn default() -> Self {
        Self {
            codec: default_codec(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SleepModeOverride {
    #[serde(default = "default_sleep_enabled")]
    pub enabled: bool,
    #[serde(default = "default_sleep_timeout")]
    pub timeout_seconds: u32,
}

impl Default for SleepModeOverride {
    fn default() -> Self {
        Self { enabled: default_sleep_enabled(), timeout_seconds: default_sleep_timeout() }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FaceTrackingOverride {
    #[serde(default = "default_ft_enabled")]
    pub enabled: bool,
    #[serde(default = "default_ft_smoothing")]
    pub smoothing: f32,
}

impl Default for FaceTrackingOverride {
    fn default() -> Self {
        Self { enabled: default_ft_enabled(), smoothing: default_ft_smoothing() }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecordingOverride {
    #[serde(default = "default_recording_enabled")]
    pub enabled: bool,
    /// Empty = %APPDATA%/FocusVisionPCVR/recordings
    #[serde(default)]
    pub output_dir: String,
}

impl Default for RecordingOverride {
    fn default() -> Self {
        Self { enabled: default_recording_enabled(), output_dir: String::new() }
    }
}

/// Audio capture/encode overrides. Mirrors the engine's `[audio]` section
/// (config/default.toml) so companion-side toggles persist across runs and
/// can later be propagated to the engine (CONFIG_UPDATE) without losing state.
#[derive(Debug, Serialize, Deserialize)]
pub struct AudioOverride {
    #[serde(default = "default_audio_enabled")]
    pub enabled: bool,
    /// Opus target bitrate, matches engine `[audio] bitrate_kbps` range 32..=512
    #[serde(default = "default_audio_bitrate")]
    pub bitrate_kbps: u32,
}

impl Default for AudioOverride {
    fn default() -> Self {
        Self { enabled: default_audio_enabled(), bitrate_kbps: default_audio_bitrate() }
    }
}

/// Deploy-tab UI state worth keeping across runs (APK path, mostly so the
/// user doesn't have to re-pick it every launch).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DeployOverride {
    #[serde(default)]
    pub apk_path: String,
}

fn default_recording_enabled() -> bool { false }
fn default_audio_enabled() -> bool { true }
fn default_audio_bitrate() -> u32 { 128 }

fn default_codec() -> String { "h265".to_string() }
fn default_sleep_enabled() -> bool { true }
fn default_sleep_timeout() -> u32 { 300 }
fn default_ft_enabled() -> bool { true }
fn default_ft_smoothing() -> f32 { 0.6 }

impl LocalConfig {
    /// Load the user's overrides from `%APPDATA%/FocusVisionPCVR/config/local.toml`.
    ///
    /// Migration: when that file does not exist yet but one written by 3.0.0
    /// or earlier is present at the legacy location (`config/local.toml` next
    /// to the exe or under the CWD), the legacy file is read instead. The next
    /// `save()` writes to the new location; the legacy file is never modified.
    pub fn load() -> Self {
        match config_path() {
            Some(path) if path.exists() => Self::load_from(&path),
            _ => match legacy_config_path() {
                Some(legacy) => {
                    log::info!("LocalConfig: migrating settings from legacy {:?}", legacy);
                    Self::load_from(&legacy)
                }
                None => Self::default(),
            },
        }
    }

    /// Load from an explicit path.
    ///
    /// A missing file is silently treated as "use defaults" (normal first-run
    /// path). A present-but-malformed file is reported via `log::warn!` and
    /// then falls back to defaults; `save_to` backs such a file up before
    /// overwriting it, so hand edits with a syntax error are never lost.
    pub(crate) fn load_from(path: &Path) -> Self {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                log::warn!("LocalConfig: cannot read {:?}: {} — using defaults", path, e);
                return Self::default();
            }
        };
        match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!(
                    "LocalConfig: failed to parse {:?}: {} — using defaults. \
                     Fix the TOML syntax to preserve user overrides.",
                    path, e
                );
                Self::default()
            }
        }
    }

    /// Save to `%APPDATA%/FocusVisionPCVR/config/local.toml`.
    ///
    /// On the first save after upgrading, the legacy file (if any) seeds the
    /// new one so hand-written keys the companion doesn't manage carry over.
    pub fn save(&self) -> Result<(), String> {
        let path = config_path().ok_or("cannot locate the user data directory")?;
        if !path.exists() {
            if let Some(legacy) = legacy_config_path() {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::copy(&legacy, &path) {
                    log::warn!("LocalConfig: cannot copy legacy {:?}: {}", legacy, e);
                }
            }
        }
        self.save_to(&path)
    }

    /// Save to an explicit path.
    ///
    /// Only the keys this struct manages are overwritten: the existing file
    /// is read as a TOML table and our values are merged into it, so unknown
    /// sections/keys (hand-written engine overrides, keys from a newer
    /// companion) survive. A file that exists but does not parse is copied to
    /// `<name>.bak` first. The write itself is atomic (temp file + rename) so
    /// a crash or a concurrent reader never sees a half-written file.
    pub(crate) fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let mut merged = match std::fs::read_to_string(path) {
            Ok(existing) => match existing.parse::<toml::Table>() {
                Ok(table) => table,
                Err(e) => {
                    let backup = path.with_extension("toml.bak");
                    std::fs::copy(path, &backup).map_err(|copy_err| {
                        format!("{:?} is not valid TOML ({e}) and could not be backed up: {copy_err}", path)
                    })?;
                    log::warn!(
                        "LocalConfig: {:?} is not valid TOML ({}); backed up to {:?} before overwriting",
                        path, e, backup
                    );
                    toml::Table::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
            // Refuse to overwrite a file we couldn't read: we'd drop whatever
            // keys it holds that we don't manage.
            Err(e) => return Err(format!("cannot read existing {:?}: {e}", path)),
        };

        let ours = toml::Table::try_from(self).map_err(|e| e.to_string())?;
        merge_tables(&mut merged, ours);
        let content = toml::to_string_pretty(&merged).map_err(|e| e.to_string())?;
        write_atomic(path, content.as_bytes())
    }
}

/// Recursively overlay `overlay` onto `base`: tables merge key by key,
/// any other value replaces the existing one.
fn merge_tables(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(existing)), toml::Value::Table(incoming)) => {
                merge_tables(existing, incoming);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Write via a sibling temp file and rename it over the target, so readers
/// see either the old or the new content, never a partial write.
fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

/// `%APPDATA%/FocusVisionPCVR/config/local.toml` — the per-user location the
/// installer promises to preserve across reinstalls (it wipes `$INSTDIR\config`,
/// which is also not writable by a standard user under Program Files) and the
/// path docs/USER_GUIDE.md points users at.
pub(crate) fn config_path() -> Option<PathBuf> {
    dirs_next::data_dir().map(|d| d.join("FocusVisionPCVR").join("config").join("local.toml"))
}

/// Legacy (3.0.0 and earlier) location: `config/local.toml` two levels above
/// the exe (dev layout) or under the CWD. Returns the first existing file.
fn legacy_config_path() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let dev_path = exe_dir.map(|d| d.join("..").join("..").join("config").join("local.toml"));
    let cwd_path = PathBuf::from("config").join("local.toml");
    dev_path.into_iter().chain([cwd_path]).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_codec_is_h265() {
        let config = LocalConfig::default();
        assert_eq!(config.video.codec, "h265");
    }

    #[test]
    fn toml_serialize_deserialize_roundtrip() {
        let mut config = LocalConfig::default();
        config.video.codec = "h264".to_string();

        let serialized = toml::to_string(&config).expect("serialize failed");
        let deserialized: LocalConfig = toml::from_str(&serialized).expect("deserialize failed");
        assert_eq!(deserialized.video.codec, "h264");
    }

    /// Unique scratch dir under the OS temp dir, removed on drop. Keeps the
    /// file-backed tests away from the real `%APPDATA%/FocusVisionPCVR`.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "fvp-companion-{tag}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("local.toml")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn config_path_is_under_user_data_dir() {
        // Regression: the file used to live under the install dir
        // (Program Files — not user-writable, wiped by the installer).
        if let Some(path) = config_path() {
            assert!(path.ends_with(std::path::Path::new("FocusVisionPCVR").join("config").join("local.toml")));
            assert!(path.starts_with(dirs_next::data_dir().unwrap()));
        }
    }

    #[test]
    fn load_from_nonexistent_file_returns_default() {
        let tmp = TempDir::new("missing");
        let config = LocalConfig::load_from(&tmp.file());
        assert_eq!(config.video.codec, "h265");
    }

    #[test]
    fn save_to_then_load_from_roundtrip_creates_parent_dir() {
        let tmp = TempDir::new("roundtrip");
        let mut config = LocalConfig::default();
        config.video.codec = "h264".to_string();
        config.sleep_mode.timeout_seconds = 120;
        config.save_to(&tmp.file()).expect("save must create the directory");

        let loaded = LocalConfig::load_from(&tmp.file());
        assert_eq!(loaded.video.codec, "h264");
        assert_eq!(loaded.sleep_mode.timeout_seconds, 120);
        // Atomic write leaves no temp file behind.
        assert!(!tmp.0.join("local.toml.tmp").exists());
    }

    #[test]
    fn save_to_preserves_unknown_sections_and_keys() {
        // Regression: saving used to rewrite the file from the struct alone,
        // silently dropping hand-written keys the companion doesn't manage.
        let tmp = TempDir::new("preserve");
        std::fs::create_dir_all(&tmp.0).unwrap();
        std::fs::write(
            tmp.file(),
            r#"
[network]
tcp_port = 9950

[video]
codec = "h264"
bitrate_mbps = 150
"#,
        )
        .unwrap();

        let mut config = LocalConfig::load_from(&tmp.file());
        assert_eq!(config.video.codec, "h264");
        config.video.codec = "h265".to_string();
        config.save_to(&tmp.file()).unwrap();

        let table: toml::Table = std::fs::read_to_string(tmp.file()).unwrap().parse().unwrap();
        assert_eq!(table["network"]["tcp_port"].as_integer(), Some(9950));
        assert_eq!(table["video"]["bitrate_mbps"].as_integer(), Some(150));
        assert_eq!(table["video"]["codec"].as_str(), Some("h265"));
        // Managed sections are still written in full.
        assert_eq!(table["audio"]["bitrate_kbps"].as_integer(), Some(128));
    }

    #[test]
    fn save_to_backs_up_malformed_file_before_overwriting() {
        // Regression: a syntax error fell back to defaults and the next save
        // overwrote the user's (half-edited) file with no way back.
        let tmp = TempDir::new("malformed");
        std::fs::create_dir_all(&tmp.0).unwrap();
        let broken = "[video\ncodec = \"h264\"\n";
        std::fs::write(tmp.file(), broken).unwrap();

        let config = LocalConfig::load_from(&tmp.file());
        assert_eq!(config.video.codec, "h265", "malformed file falls back to defaults");
        config.save_to(&tmp.file()).unwrap();

        let backup = tmp.0.join("local.toml.bak");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), broken);
        let saved: LocalConfig =
            toml::from_str(&std::fs::read_to_string(tmp.file()).unwrap()).unwrap();
        assert_eq!(saved.video.codec, "h265");
    }

    #[test]
    fn merge_tables_overrides_scalars_and_recurses_into_tables() {
        let mut base: toml::Table = "a = 1\n[s]\nkeep = true\nx = 1\n".parse().unwrap();
        let overlay: toml::Table = "a = 2\n[s]\nx = 3\n[t]\ny = 4\n".parse().unwrap();
        merge_tables(&mut base, overlay);
        assert_eq!(base["a"].as_integer(), Some(2));
        assert_eq!(base["s"]["keep"].as_bool(), Some(true));
        assert_eq!(base["s"]["x"].as_integer(), Some(3));
        assert_eq!(base["t"]["y"].as_integer(), Some(4));
    }

    #[test]
    fn save_then_load_preserves_codec_change() {
        // Test serialize then deserialize via toml strings (avoids file path dependency)
        let mut config = LocalConfig::default();
        config.video.codec = "h264".to_string();

        let serialized = toml::to_string_pretty(&config).expect("serialize failed");
        let loaded: LocalConfig = toml::from_str(&serialized).expect("deserialize failed");
        assert_eq!(loaded.video.codec, "h264");
    }

    #[test]
    fn invalid_toml_content_falls_back_to_default() {
        let result: Result<LocalConfig, _> = toml::from_str("this is {{not valid toml!!");
        let config = result.unwrap_or_default();
        assert_eq!(config.video.codec, "h265");
    }

    #[test]
    fn empty_string_toml_falls_back_to_default() {
        let config: LocalConfig = toml::from_str("").expect("empty string should parse as default");
        assert_eq!(config.video.codec, "h265");
    }

    #[test]
    fn recording_default_is_disabled_with_empty_dir() {
        let config = LocalConfig::default();
        assert!(!config.recording.enabled);
        assert!(config.recording.output_dir.is_empty());
    }

    #[test]
    fn recording_override_roundtrip() {
        let mut config = LocalConfig::default();
        config.recording.enabled = true;
        config.recording.output_dir = "D:/captures".to_string();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: LocalConfig = toml::from_str(&serialized).unwrap();
        assert!(deserialized.recording.enabled);
        assert_eq!(deserialized.recording.output_dir, "D:/captures");
    }

    #[test]
    fn audio_default_matches_engine_defaults() {
        let config = LocalConfig::default();
        assert!(config.audio.enabled);
        assert_eq!(config.audio.bitrate_kbps, 128);
    }

    #[test]
    fn audio_override_roundtrip() {
        let mut config = LocalConfig::default();
        config.audio.enabled = false;
        config.audio.bitrate_kbps = 64;
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: LocalConfig = toml::from_str(&serialized).unwrap();
        assert!(!deserialized.audio.enabled);
        assert_eq!(deserialized.audio.bitrate_kbps, 64);
    }

    #[test]
    fn audio_section_missing_falls_back_to_default() {
        // Older local.toml files won't have [audio] yet; the section is opt-in
        // via #[serde(default)] so loading must still succeed with engine defaults.
        let older_toml = r#"
[video]
codec = "h264"
"#;
        let config: LocalConfig = toml::from_str(older_toml).expect("should parse");
        assert_eq!(config.video.codec, "h264");
        assert!(config.audio.enabled);
        assert_eq!(config.audio.bitrate_kbps, 128);
    }

    #[test]
    fn deploy_apk_path_roundtrip() {
        let mut config = LocalConfig::default();
        config.deploy.apk_path = r"C:\builds\fvp-client-debug.apk".to_string();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: LocalConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(
            deserialized.deploy.apk_path,
            r"C:\builds\fvp-client-debug.apk"
        );
    }
}
