use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User-local config overrides (config/local.toml, gitignored).
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
    /// Load from config/local.toml relative to the project root.
    ///
    /// A missing file is silently treated as "use defaults" (normal first-run
    /// path). A present-but-malformed file is reported via `log::warn!` and
    /// then falls back to defaults, so users editing local.toml by hand can
    /// see syntax errors in the companion log without the app silently
    /// discarding their changes.
    pub fn load() -> Self {
        let path = config_path();
        let content = match std::fs::read_to_string(&path) {
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

    /// Save to config/local.toml.
    pub fn save(&self) -> Result<(), String> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }
}

fn config_path() -> PathBuf {
    // Look for config/ directory relative to executable
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    // Try exe_dir/../../config/local.toml (typical dev layout)
    let dev_path = exe_dir
        .join("..")
        .join("..")
        .join("config")
        .join("local.toml");
    if dev_path.parent().is_some_and(|p| p.exists()) {
        return dev_path;
    }

    // Fallback: config/local.toml relative to CWD
    PathBuf::from("config").join("local.toml")
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

    #[test]
    fn load_from_nonexistent_file_returns_default() {
        // load() will fail to read a file and fall back to default
        let config = LocalConfig::load();
        assert_eq!(config.video.codec, "h265");
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
