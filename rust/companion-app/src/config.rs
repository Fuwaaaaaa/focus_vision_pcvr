use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User-local config overrides (config/local.toml, gitignored).
/// Only contains fields the user explicitly changed via the companion UI.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub video: VideoOverride,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct VideoOverride {
    /// "h265" or "h264"
    #[serde(default = "default_codec")]
    pub codec: String,
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
