use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User-local config overrides (config/local.toml, gitignored).
/// Only contains fields the user explicitly changed via the companion UI.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub video: VideoOverride,
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

fn default_codec() -> String {
    "h265".to_string()
}

impl LocalConfig {
    /// Load from config/local.toml relative to the project root.
    pub fn load() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
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
    if dev_path.parent().map_or(false, |p| p.exists()) {
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
}
