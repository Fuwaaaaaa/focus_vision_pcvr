use serde::{Deserialize, Serialize};
use fvp_common::protocol::VideoCodec;

/// Structured config validation error. Values are clamped to defaults (graceful migration).
#[derive(Debug, Clone)]
pub struct ConfigError {
    pub field: &'static str,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Config validation [{}]: {}", self.field, self.message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct AppConfig {
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub pairing: PairingConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub foveated: FoveatedConfig,
    #[serde(default)]
    pub face_tracking: FaceTrackingConfig,
    #[serde(default)]
    pub sleep_mode: SleepModeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_audio_enabled")]
    pub enabled: bool,
    #[serde(default = "default_audio_bitrate")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_audio_frame_size")]
    pub frame_size_ms: u32,
    #[serde(default = "default_audio_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_audio_channels")]
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    #[serde(default = "default_ipd")]
    pub ipd: f32,
    #[serde(default = "default_vsync_to_photons")]
    pub seconds_from_vsync_to_photons: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_tcp_port")]
    pub tcp_port: u16,
    #[serde(default = "default_udp_port")]
    pub udp_port: u16,
    #[serde(default = "default_fec_redundancy")]
    pub fec_redundancy: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoConfig {
    #[serde(default)]
    pub codec: VideoCodec,
    #[serde(default = "default_bitrate")]
    pub bitrate_mbps: u32,
    #[serde(default = "default_resolution")]
    pub resolution_per_eye: [u32; 2],
    #[serde(default = "default_framerate")]
    pub framerate: u32,
    /// Full RGB color range (0-255) instead of limited (16-235).
    /// Requires NVENC VUI parameter support — see TODOS.md for SDK verification.
    #[serde(default = "default_full_range")]
    pub full_range: bool,
}

fn default_full_range() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u8,
    #[serde(default = "default_lockout_seconds")]
    pub lockout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoveatedConfig {
    #[serde(default = "default_foveated_enabled")]
    pub enabled: bool,
    #[serde(default = "default_fovea_radius")]
    pub fovea_radius: f32,
    #[serde(default = "default_mid_radius")]
    pub mid_radius: f32,
    #[serde(default = "default_mid_qp_offset")]
    pub mid_qp_offset: i32,
    #[serde(default = "default_peripheral_qp_offset")]
    pub peripheral_qp_offset: i32,
    /// Foveated encoding preset: "subtle", "balanced", "aggressive".
    /// Overrides mid_qp_offset and peripheral_qp_offset when set.
    #[serde(default = "default_foveated_preset")]
    pub preset: FoveatedPreset,
}

/// Foveated encoding preset with predefined QP offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FoveatedPreset {
    /// Subtle: mid +3, peripheral +8 (minimal quality difference)
    Subtle,
    /// Balanced: mid +5, peripheral +15 (default, ~20% bandwidth reduction)
    #[default]
    Balanced,
    /// Aggressive: mid +8, peripheral +25 (~35% bandwidth reduction)
    Aggressive,
    /// Custom: use mid_qp_offset and peripheral_qp_offset values directly
    Custom,
}


impl FoveatedPreset {
    /// Get QP offsets for this preset. Returns (mid, peripheral).
    pub fn qp_offsets(self) -> Option<(i32, i32)> {
        match self {
            Self::Subtle => Some((3, 8)),
            Self::Balanced => Some((5, 15)),
            Self::Aggressive => Some((8, 25)),
            Self::Custom => None, // Use config values directly
        }
    }
}

fn default_foveated_enabled() -> bool { false }
fn default_fovea_radius() -> f32 { 0.15 }
fn default_mid_radius() -> f32 { 0.35 }
fn default_mid_qp_offset() -> i32 { 5 }
fn default_peripheral_qp_offset() -> i32 { 15 }
fn default_foveated_preset() -> FoveatedPreset { FoveatedPreset::Balanced }

impl Default for FoveatedConfig {
    fn default() -> Self {
        Self {
            enabled: default_foveated_enabled(),
            fovea_radius: default_fovea_radius(),
            mid_radius: default_mid_radius(),
            mid_qp_offset: default_mid_qp_offset(),
            peripheral_qp_offset: default_peripheral_qp_offset(),
            preset: default_foveated_preset(),
        }
    }
}

impl FoveatedConfig {
    /// Get effective QP offsets (preset overrides manual values unless Custom).
    pub fn effective_qp_offsets(&self) -> (i32, i32) {
        self.preset.qp_offsets().unwrap_or((self.mid_qp_offset, self.peripheral_qp_offset))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceTrackingConfig {
    #[serde(default = "default_ft_enabled")]
    pub enabled: bool,
    #[serde(default = "default_ft_smoothing")]
    pub smoothing: f32,
    #[serde(default = "default_ft_osc_port")]
    pub osc_port: u16,
    /// Active expression profile name. Empty = no profile (all weights 1.0).
    #[serde(default)]
    pub active_profile: String,
}

impl Default for FaceTrackingConfig {
    fn default() -> Self {
        Self {
            enabled: default_ft_enabled(),
            smoothing: default_ft_smoothing(),
            osc_port: default_ft_osc_port(),
            active_profile: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleepModeConfig {
    #[serde(default = "default_sleep_enabled")]
    pub enabled: bool,
    #[serde(default = "default_sleep_timeout")]
    pub timeout_seconds: u32,
    #[serde(default = "default_sleep_motion_threshold")]
    pub motion_threshold: f32,
    #[serde(default = "default_sleep_bitrate")]
    pub sleep_bitrate_mbps: u32,
}

impl Default for SleepModeConfig {
    fn default() -> Self {
        Self {
            enabled: default_sleep_enabled(),
            timeout_seconds: default_sleep_timeout(),
            motion_threshold: default_sleep_motion_threshold(),
            sleep_bitrate_mbps: default_sleep_bitrate(),
        }
    }
}

fn default_sleep_enabled() -> bool { true }
fn default_sleep_timeout() -> u32 { 300 }
fn default_sleep_motion_threshold() -> f32 { 0.002 }
fn default_sleep_bitrate() -> u32 { 8 }

fn default_ft_enabled() -> bool { true }
fn default_ft_smoothing() -> f32 { 0.6 }
fn default_ft_osc_port() -> u16 { 9000 }

fn default_tcp_port() -> u16 { fvp_common::DEFAULT_TCP_PORT }
fn default_udp_port() -> u16 { fvp_common::DEFAULT_UDP_PORT }
fn default_fec_redundancy() -> f32 { fvp_common::DEFAULT_FEC_REDUNDANCY }
fn default_bitrate() -> u32 { 80 }
fn default_resolution() -> [u32; 2] { [1832, 1920] }
fn default_framerate() -> u32 { 90 }
fn default_ipd() -> f32 { 0.063 }
fn default_vsync_to_photons() -> f32 { 0.011 }
fn default_audio_enabled() -> bool { true }
fn default_audio_bitrate() -> u32 { 128 }
fn default_audio_frame_size() -> u32 { 10 }
fn default_audio_sample_rate() -> u32 { 48000 }
fn default_audio_channels() -> u16 { 2 }
fn default_max_attempts() -> u8 { fvp_common::MAX_PIN_ATTEMPTS }
fn default_lockout_seconds() -> u64 { fvp_common::PIN_LOCKOUT_SECONDS }

impl Default for NetworkConfig {
    fn default() -> Self {
        Self { tcp_port: default_tcp_port(), udp_port: default_udp_port(), fec_redundancy: default_fec_redundancy() }
    }
}
impl Default for VideoConfig {
    fn default() -> Self {
        Self { codec: VideoCodec::default(), bitrate_mbps: default_bitrate(), resolution_per_eye: default_resolution(), framerate: default_framerate(), full_range: default_full_range() }
    }
}
impl Default for PairingConfig {
    fn default() -> Self {
        Self { max_attempts: default_max_attempts(), lockout_seconds: default_lockout_seconds() }
    }
}
impl Default for DisplayConfig {
    fn default() -> Self {
        Self { ipd: default_ipd(), seconds_from_vsync_to_photons: default_vsync_to_photons() }
    }
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: default_audio_enabled(),
            bitrate_kbps: default_audio_bitrate(),
            frame_size_ms: default_audio_frame_size(),
            sample_rate: default_audio_sample_rate(),
            channels: default_audio_channels(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    /// Validate config values, returning structured errors for any corrected fields.
    /// Invalid values are clamped to valid defaults (graceful migration).
    /// Callers should log errors and can inspect which fields were corrected.
    pub fn validate(&mut self) -> Vec<ConfigError> {
        let mut errors = Vec::new();

        // Network
        if self.network.tcp_port < 1024 {
            errors.push(ConfigError { field: "network.tcp_port", message: format!("{} < 1024, clamped to {}", self.network.tcp_port, default_tcp_port()) });
            self.network.tcp_port = default_tcp_port();
        }
        if self.network.udp_port < 1024 {
            errors.push(ConfigError { field: "network.udp_port", message: format!("{} < 1024, clamped to {}", self.network.udp_port, default_udp_port()) });
            self.network.udp_port = default_udp_port();
        }
        if self.network.tcp_port == self.network.udp_port {
            errors.push(ConfigError { field: "network.udp_port", message: format!("== tcp_port ({}), offsetting", self.network.tcp_port) });
            self.network.udp_port = self.network.tcp_port + 1;
        }

        // Video
        if self.video.bitrate_mbps < 10 || self.video.bitrate_mbps > 200 {
            errors.push(ConfigError { field: "video.bitrate_mbps", message: format!("{} out of range [10-200], clamped to 80", self.video.bitrate_mbps) });
            self.video.bitrate_mbps = 80;
        }
        if self.video.framerate < 30 || self.video.framerate > 120 {
            errors.push(ConfigError { field: "video.framerate", message: format!("{} out of range [30-120], clamped to 90", self.video.framerate) });
            self.video.framerate = 90;
        }

        // Face tracking
        if self.face_tracking.smoothing.is_nan() || self.face_tracking.smoothing.is_infinite()
            || self.face_tracking.smoothing < 0.0 || self.face_tracking.smoothing > 0.99
        {
            errors.push(ConfigError { field: "face_tracking.smoothing", message: format!("{} invalid, clamped to 0.6", self.face_tracking.smoothing) });
            self.face_tracking.smoothing = 0.6;
        }

        // Sleep mode
        if self.sleep_mode.timeout_seconds < 30 || self.sleep_mode.timeout_seconds > 3600 {
            errors.push(ConfigError { field: "sleep_mode.timeout_seconds", message: format!("{} out of range [30-3600], clamped to 300", self.sleep_mode.timeout_seconds) });
            self.sleep_mode.timeout_seconds = 300;
        }
        if self.sleep_mode.motion_threshold <= 0.0 || self.sleep_mode.motion_threshold > 0.1
            || self.sleep_mode.motion_threshold.is_nan()
        {
            errors.push(ConfigError { field: "sleep_mode.motion_threshold", message: format!("{} invalid, clamped to 0.002", self.sleep_mode.motion_threshold) });
            self.sleep_mode.motion_threshold = 0.002;
        }

        // Foveated
        if self.foveated.fovea_radius <= 0.0 || self.foveated.fovea_radius > 0.5
            || self.foveated.fovea_radius.is_nan()
        {
            errors.push(ConfigError { field: "foveated.fovea_radius", message: format!("{} invalid, clamped to 0.15", self.foveated.fovea_radius) });
            self.foveated.fovea_radius = 0.15;
        }
        if self.foveated.mid_radius <= self.foveated.fovea_radius || self.foveated.mid_radius > 1.0
            || self.foveated.mid_radius.is_nan()
        {
            errors.push(ConfigError { field: "foveated.mid_radius", message: format!("{} invalid (must be > fovea_radius), clamped to 0.35", self.foveated.mid_radius) });
            self.foveated.mid_radius = 0.35;
        }

        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.network.tcp_port, 9944);
        assert_eq!(cfg.network.udp_port, 9945);
        assert_eq!(cfg.video.bitrate_mbps, 80);
        assert_eq!(cfg.video.resolution_per_eye, [1832, 1920]);
        assert_eq!(cfg.video.framerate, 90);
        assert_eq!(cfg.audio.sample_rate, 48000);
        assert_eq!(cfg.pairing.max_attempts, 5);
    }

    #[test]
    fn test_load_default_toml() {
        // Try from workspace root or streaming-engine dir
        let path = if std::path::Path::new("config/default.toml").exists() {
            "config/default.toml"
        } else if std::path::Path::new("../../config/default.toml").exists() {
            "../../config/default.toml"
        } else {
            // Skip if config not found (CI may not have it)
            return;
        };
        let cfg = AppConfig::load(path);
        assert!(cfg.is_ok(), "default.toml should parse: {:?}", cfg.err());
        let cfg = cfg.unwrap();
        assert_eq!(cfg.video.bitrate_mbps, 80);
        assert_eq!(cfg.network.fec_redundancy, 0.2);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = AppConfig::load("nonexistent.toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_toml() {
        let cfg: AppConfig = toml::from_str("").unwrap();
        // All fields should fall back to defaults
        assert_eq!(cfg.video.framerate, 90);
        assert_eq!(cfg.network.tcp_port, 9944);
    }

    #[test]
    fn test_parse_partial_toml() {
        let cfg: AppConfig = toml::from_str(r#"
            [video]
            bitrate_mbps = 120
        "#).unwrap();
        assert_eq!(cfg.video.bitrate_mbps, 120);
        // Other fields should be defaults
        assert_eq!(cfg.video.framerate, 90);
        assert_eq!(cfg.network.tcp_port, 9944);
    }

    #[test]
    fn test_validate_default_config_is_clean() {
        let mut cfg = AppConfig::default();
        let errors = cfg.validate();
        assert!(errors.is_empty(), "Default config should have no errors: {:?}", errors);
    }

    #[test]
    fn test_validate_bitrate_out_of_range() {
        let mut cfg = AppConfig::default();
        cfg.video.bitrate_mbps = 0;
        let errors = cfg.validate();
        assert!(!errors.is_empty());
        assert_eq!(cfg.video.bitrate_mbps, 80); // clamped to default
        assert_eq!(errors[0].field, "video.bitrate_mbps");
    }

    #[test]
    fn test_validate_port_too_low() {
        let mut cfg = AppConfig::default();
        cfg.network.tcp_port = 80;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.tcp_port"));
        assert_eq!(cfg.network.tcp_port, default_tcp_port());
    }

    #[test]
    fn test_validate_port_conflict() {
        let mut cfg = AppConfig::default();
        cfg.network.tcp_port = 5000;
        cfg.network.udp_port = 5000;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.udp_port"));
        assert_ne!(cfg.network.tcp_port, cfg.network.udp_port);
    }

    #[test]
    fn test_validate_smoothing_nan() {
        let mut cfg = AppConfig::default();
        cfg.face_tracking.smoothing = f32::NAN;
        let errors = cfg.validate();
        assert!(!errors.is_empty());
        assert_eq!(cfg.face_tracking.smoothing, 0.6);
        assert_eq!(errors[0].field, "face_tracking.smoothing");
    }

    #[test]
    fn test_validate_sleep_timeout_zero() {
        let mut cfg = AppConfig::default();
        cfg.sleep_mode.timeout_seconds = 0;
        let errors = cfg.validate();
        assert!(!errors.is_empty());
        assert_eq!(cfg.sleep_mode.timeout_seconds, 300);
        assert_eq!(errors[0].field, "sleep_mode.timeout_seconds");
    }

    #[test]
    fn test_validate_accepts_edge_values() {
        let mut cfg = AppConfig::default();
        cfg.video.bitrate_mbps = 10; // min
        cfg.video.framerate = 120; // max
        cfg.sleep_mode.timeout_seconds = 3600; // max
        cfg.face_tracking.smoothing = 0.0; // min
        let errors = cfg.validate();
        assert!(errors.is_empty(), "Edge values should be valid: {:?}", errors);
    }
}
