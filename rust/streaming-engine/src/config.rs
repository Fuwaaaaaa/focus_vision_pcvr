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
    #[serde(default)]
    pub memory_monitor: MemoryMonitorConfig,
    #[serde(default)]
    pub recording: RecordingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    /// Enable session recording (writes raw Annex B H.264/H.265 to disk).
    #[serde(default = "default_recording_enabled")]
    pub enabled: bool,
    /// Output directory. Empty string = %APPDATA%/FocusVisionPCVR/recordings.
    #[serde(default)]
    pub output_dir: String,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            enabled: default_recording_enabled(),
            output_dir: String::new(),
        }
    }
}

fn default_recording_enabled() -> bool { false }

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
    #[serde(default = "default_fec_redundancy_min")]
    pub fec_redundancy_min: f32,
    #[serde(default = "default_fec_redundancy_max")]
    pub fec_redundancy_max: f32,
    #[serde(default = "default_adaptive_fec_enabled")]
    pub adaptive_fec_enabled: bool,
    #[serde(default = "default_congestion_control")]
    pub congestion_control: String,
    #[serde(default = "default_slice_fec_enabled")]
    pub slice_fec_enabled: bool,
    #[serde(default = "default_slice_count")]
    pub slice_count: u8,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMonitorConfig {
    #[serde(default = "default_memory_monitor_enabled")]
    pub enabled: bool,
    #[serde(default = "default_memory_poll_seconds")]
    pub poll_interval_seconds: u32,
    /// Warn if process memory grows by this many MB within 1 hour.
    #[serde(default = "default_memory_growth_threshold_mb")]
    pub growth_threshold_mb: u32,
}

impl Default for MemoryMonitorConfig {
    fn default() -> Self {
        Self {
            enabled: default_memory_monitor_enabled(),
            poll_interval_seconds: default_memory_poll_seconds(),
            growth_threshold_mb: default_memory_growth_threshold_mb(),
        }
    }
}

fn default_memory_monitor_enabled() -> bool { true }
fn default_memory_poll_seconds() -> u32 { 60 }
fn default_memory_growth_threshold_mb() -> u32 { 50 }
fn default_adaptive_fec_enabled() -> bool { true }
fn default_congestion_control() -> String { "gcc".to_string() }
fn default_slice_fec_enabled() -> bool { true }
fn default_slice_count() -> u8 { 4 }

fn default_ft_enabled() -> bool { true }
fn default_ft_smoothing() -> f32 { 0.6 }
fn default_ft_osc_port() -> u16 { 9000 }

fn default_tcp_port() -> u16 { fvp_common::DEFAULT_TCP_PORT }
fn default_udp_port() -> u16 { fvp_common::DEFAULT_UDP_PORT }
fn default_fec_redundancy() -> f32 { fvp_common::DEFAULT_FEC_REDUNDANCY }
fn default_fec_redundancy_min() -> f32 { 0.05 }
fn default_fec_redundancy_max() -> f32 { 0.40 }
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
        Self {
            tcp_port: default_tcp_port(),
            udp_port: default_udp_port(),
            fec_redundancy: default_fec_redundancy(),
            fec_redundancy_min: default_fec_redundancy_min(),
            fec_redundancy_max: default_fec_redundancy_max(),
            adaptive_fec_enabled: default_adaptive_fec_enabled(),
            congestion_control: default_congestion_control(),
            slice_fec_enabled: default_slice_fec_enabled(),
            slice_count: default_slice_count(),
        }
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

        // FEC redundancy
        if self.network.fec_redundancy_min < 0.0 || self.network.fec_redundancy_min > 1.0
            || self.network.fec_redundancy_min.is_nan()
        {
            errors.push(ConfigError { field: "network.fec_redundancy_min", message: format!("{} invalid, clamped to 0.05", self.network.fec_redundancy_min) });
            self.network.fec_redundancy_min = 0.05;
        }
        if self.network.fec_redundancy_max < self.network.fec_redundancy_min
            || self.network.fec_redundancy_max > 1.0
            || self.network.fec_redundancy_max.is_nan()
        {
            errors.push(ConfigError { field: "network.fec_redundancy_max", message: format!("{} invalid, clamped to 0.40", self.network.fec_redundancy_max) });
            self.network.fec_redundancy_max = 0.40;
        }
        // Validate initial fec_redundancy is within [min, max]
        if self.network.fec_redundancy.is_nan()
            || self.network.fec_redundancy < self.network.fec_redundancy_min
            || self.network.fec_redundancy > self.network.fec_redundancy_max
        {
            let clamped = self.network.fec_redundancy.clamp(
                self.network.fec_redundancy_min, self.network.fec_redundancy_max
            );
            errors.push(ConfigError { field: "network.fec_redundancy", message: format!("{} out of [{}, {}], clamped to {}", self.network.fec_redundancy, self.network.fec_redundancy_min, self.network.fec_redundancy_max, clamped) });
            self.network.fec_redundancy = clamped;
        }

        // Congestion control mode
        if self.network.congestion_control != "gcc" && self.network.congestion_control != "loss" {
            errors.push(ConfigError {
                field: "network.congestion_control",
                message: format!("must be \"gcc\" or \"loss\", got \"{}\"", self.network.congestion_control),
            });
            self.network.congestion_control = "gcc".to_string();
        }

        // Slice FEC
        validate_range(&mut self.network.slice_count, 2, 15, 4, "network.slice_count", &mut errors);

        // Video
        validate_range(&mut self.video.bitrate_mbps, 10, 200, 80, "video.bitrate_mbps", &mut errors);
        validate_range(&mut self.video.framerate, 30, 120, 90, "video.framerate", &mut errors);

        // Face tracking
        validate_f32_range(&mut self.face_tracking.smoothing, 0.0, 0.99, 0.6, "face_tracking.smoothing", &mut errors);

        // Sleep mode
        validate_range(&mut self.sleep_mode.timeout_seconds, 30, 3600, 300, "sleep_mode.timeout_seconds", &mut errors);
        // motion_threshold: min is exclusive (> 0.0) — keep manual check
        if self.sleep_mode.motion_threshold <= 0.0 || self.sleep_mode.motion_threshold > 0.1
            || self.sleep_mode.motion_threshold.is_nan()
        {
            errors.push(ConfigError { field: "sleep_mode.motion_threshold", message: format!("{} invalid, clamped to 0.002", self.sleep_mode.motion_threshold) });
            self.sleep_mode.motion_threshold = 0.002;
        }

        // Audio
        if self.audio.sample_rate != 48000 {
            errors.push(ConfigError { field: "audio.sample_rate", message: format!("{} unsupported, clamped to 48000", self.audio.sample_rate) });
            self.audio.sample_rate = 48000;
        }
        validate_range(&mut self.audio.bitrate_kbps, 32, 512, 128, "audio.bitrate_kbps", &mut errors);

        // Foveated
        // fovea_radius: min is exclusive (> 0.0) — keep manual check
        if self.foveated.fovea_radius <= 0.0 || self.foveated.fovea_radius > 0.5
            || self.foveated.fovea_radius.is_nan()
        {
            errors.push(ConfigError { field: "foveated.fovea_radius", message: format!("{} invalid, clamped to 0.15", self.foveated.fovea_radius) });
            self.foveated.fovea_radius = 0.15;
        }
        // mid_radius: depends on fovea_radius — keep manual check
        if self.foveated.mid_radius <= self.foveated.fovea_radius || self.foveated.mid_radius > 1.0
            || self.foveated.mid_radius.is_nan()
        {
            errors.push(ConfigError { field: "foveated.mid_radius", message: format!("{} invalid (must be > fovea_radius), clamped to 0.35", self.foveated.mid_radius) });
            self.foveated.mid_radius = 0.35;
        }
        validate_range(&mut self.foveated.mid_qp_offset, 0, 51, 5, "foveated.mid_qp_offset", &mut errors);
        validate_range(&mut self.foveated.peripheral_qp_offset, 0, 51, 10, "foveated.peripheral_qp_offset", &mut errors);

        errors
    }
}

/// Range check + clamp helper for comparable values. If `*value` is outside
/// [min, max], it is replaced with `default` and an error is pushed.
fn validate_range<T>(
    value: &mut T, min: T, max: T, default: T,
    field: &'static str, errors: &mut Vec<ConfigError>,
)
where
    T: PartialOrd + std::fmt::Display + Copy,
{
    if *value < min || *value > max {
        errors.push(ConfigError {
            field,
            message: format!("{} out of range [{}-{}], clamped to {}", value, min, max, default),
        });
        *value = default;
    }
}

/// f32-specialized range check: also rejects NaN and Infinity. Intended for
/// cases where min is inclusive; for exclusive-min checks, keep a manual block.
fn validate_f32_range(
    value: &mut f32, min: f32, max: f32, default: f32,
    field: &'static str, errors: &mut Vec<ConfigError>,
) {
    if value.is_nan() || value.is_infinite() || *value < min || *value > max {
        errors.push(ConfigError {
            field,
            message: format!("{} invalid or out of [{}, {}], clamped to {}", value, min, max, default),
        });
        *value = default;
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

    #[test]
    fn test_validate_audio_sample_rate() {
        let mut cfg = AppConfig::default();
        cfg.audio.sample_rate = 44100; // unsupported
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "audio.sample_rate"));
        assert_eq!(cfg.audio.sample_rate, 48000);
    }

    #[test]
    fn test_validate_audio_bitrate_out_of_range() {
        let mut cfg = AppConfig::default();
        cfg.audio.bitrate_kbps = 1000; // too high
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "audio.bitrate_kbps"));
        assert_eq!(cfg.audio.bitrate_kbps, 128);

        cfg.audio.bitrate_kbps = 16; // too low
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "audio.bitrate_kbps"));
        assert_eq!(cfg.audio.bitrate_kbps, 128);
    }

    #[test]
    fn test_validate_fec_redundancy_min_nan() {
        let mut cfg = AppConfig::default();
        cfg.network.fec_redundancy_min = f32::NAN;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.fec_redundancy_min"));
        assert!((cfg.network.fec_redundancy_min - 0.05).abs() < 0.01);
    }

    #[test]
    fn test_validate_fec_redundancy_min_negative() {
        let mut cfg = AppConfig::default();
        cfg.network.fec_redundancy_min = -0.1;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.fec_redundancy_min"));
        assert!((cfg.network.fec_redundancy_min - 0.05).abs() < 0.01);
    }

    #[test]
    fn test_validate_fec_redundancy_max_less_than_min() {
        let mut cfg = AppConfig::default();
        cfg.network.fec_redundancy_min = 0.10;
        cfg.network.fec_redundancy_max = 0.05; // max < min
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.fec_redundancy_max"));
        assert!((cfg.network.fec_redundancy_max - 0.40).abs() < 0.01);
    }

    #[test]
    fn test_validate_fec_redundancy_valid_range() {
        let mut cfg = AppConfig::default();
        cfg.network.fec_redundancy_min = 0.05;
        cfg.network.fec_redundancy_max = 0.40;
        let errors = cfg.validate();
        assert!(!errors.iter().any(|e| e.field.contains("fec_redundancy")));
    }

    #[test]
    fn test_validate_fec_redundancy_outside_min_max() {
        let mut cfg = AppConfig::default();
        cfg.network.fec_redundancy_min = 0.30;
        cfg.network.fec_redundancy_max = 0.40;
        cfg.network.fec_redundancy = 0.20; // below min
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.fec_redundancy"));
        assert!((cfg.network.fec_redundancy - 0.30).abs() < 0.01); // clamped to min
    }

    #[test]
    fn test_validate_fec_redundancy_nan() {
        let mut cfg = AppConfig::default();
        cfg.network.fec_redundancy = f32::NAN;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "network.fec_redundancy"));
    }

    #[test]
    fn test_validate_qp_offset_negative() {
        let mut cfg = AppConfig::default();
        cfg.foveated.mid_qp_offset = -1;
        cfg.foveated.peripheral_qp_offset = -100;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "foveated.mid_qp_offset"));
        assert!(errors.iter().any(|e| e.field == "foveated.peripheral_qp_offset"));
        assert_eq!(cfg.foveated.mid_qp_offset, 5);
        assert_eq!(cfg.foveated.peripheral_qp_offset, 10);
    }

    #[test]
    fn test_validate_qp_offset_too_high() {
        let mut cfg = AppConfig::default();
        cfg.foveated.mid_qp_offset = 52;
        cfg.foveated.peripheral_qp_offset = 1000;
        let errors = cfg.validate();
        assert!(errors.iter().any(|e| e.field == "foveated.mid_qp_offset"));
        assert!(errors.iter().any(|e| e.field == "foveated.peripheral_qp_offset"));
        assert_eq!(cfg.foveated.mid_qp_offset, 5);
        assert_eq!(cfg.foveated.peripheral_qp_offset, 10);
    }

    #[test]
    fn test_validate_qp_offset_valid_boundaries() {
        let mut cfg = AppConfig::default();
        cfg.foveated.mid_qp_offset = 0;
        cfg.foveated.peripheral_qp_offset = 51;
        let errors = cfg.validate();
        assert!(!errors.iter().any(|e| e.field == "foveated.mid_qp_offset"));
        assert!(!errors.iter().any(|e| e.field == "foveated.peripheral_qp_offset"));
        assert_eq!(cfg.foveated.mid_qp_offset, 0);
        assert_eq!(cfg.foveated.peripheral_qp_offset, 51);
    }

    #[test]
    fn test_default_congestion_control() {
        let config = AppConfig::default();
        assert_eq!(config.network.congestion_control, "gcc");
    }

    #[test]
    fn test_validate_congestion_control_invalid() {
        let mut config = AppConfig::default();
        config.network.congestion_control = "invalid".to_string();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.field == "network.congestion_control"));
        assert_eq!(config.network.congestion_control, "gcc");
    }

    #[test]
    fn test_validate_congestion_control_loss() {
        let mut config = AppConfig::default();
        config.network.congestion_control = "loss".to_string();
        let errors = config.validate();
        assert!(!errors.iter().any(|e| e.field == "network.congestion_control"));
    }

    #[test]
    fn test_default_slice_fec() {
        let config = AppConfig::default();
        assert!(config.network.slice_fec_enabled);
        assert_eq!(config.network.slice_count, 4);
    }

    #[test]
    fn test_validate_slice_count_out_of_range() {
        let mut config = AppConfig::default();
        config.network.slice_count = 1; // below 2
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.field == "network.slice_count"));
        assert_eq!(config.network.slice_count, 4); // clamped

        let mut config2 = AppConfig::default();
        config2.network.slice_count = 16; // above 15
        let errors2 = config2.validate();
        assert!(errors2.iter().any(|e| e.field == "network.slice_count"));
        assert_eq!(config2.network.slice_count, 4);
    }

    #[test]
    fn test_validate_slice_count_valid() {
        let mut config = AppConfig::default();
        config.network.slice_count = 8;
        let errors = config.validate();
        assert!(!errors.iter().any(|e| e.field == "network.slice_count"));
        assert_eq!(config.network.slice_count, 8);
    }

    /// Catches drift when a new field is added with `#[serde(default = "...")]`
    /// but the corresponding manual `impl Default for *Config` is not updated
    /// (or vice versa). Empty TOML → each field hits its serde default fn;
    /// `AppConfig::default()` → each field hits its `Default::default()`.
    /// The two paths must agree field-by-field.
    #[test]
    fn test_default_matches_empty_toml_parse() {
        let from_default = AppConfig::default();
        let from_serde: AppConfig = toml::from_str("").expect("empty TOML must parse");

        // network
        assert_eq!(from_default.network.tcp_port, from_serde.network.tcp_port);
        assert_eq!(from_default.network.udp_port, from_serde.network.udp_port);
        assert_eq!(from_default.network.fec_redundancy, from_serde.network.fec_redundancy);
        assert_eq!(from_default.network.fec_redundancy_min, from_serde.network.fec_redundancy_min);
        assert_eq!(from_default.network.fec_redundancy_max, from_serde.network.fec_redundancy_max);
        assert_eq!(from_default.network.adaptive_fec_enabled, from_serde.network.adaptive_fec_enabled);
        assert_eq!(from_default.network.congestion_control, from_serde.network.congestion_control);
        assert_eq!(from_default.network.slice_fec_enabled, from_serde.network.slice_fec_enabled);
        assert_eq!(from_default.network.slice_count, from_serde.network.slice_count);
        // video
        assert_eq!(from_default.video.bitrate_mbps, from_serde.video.bitrate_mbps);
        assert_eq!(from_default.video.resolution_per_eye, from_serde.video.resolution_per_eye);
        assert_eq!(from_default.video.framerate, from_serde.video.framerate);
        assert_eq!(from_default.video.full_range, from_serde.video.full_range);
        // audio
        assert_eq!(from_default.audio.enabled, from_serde.audio.enabled);
        assert_eq!(from_default.audio.bitrate_kbps, from_serde.audio.bitrate_kbps);
        assert_eq!(from_default.audio.frame_size_ms, from_serde.audio.frame_size_ms);
        assert_eq!(from_default.audio.sample_rate, from_serde.audio.sample_rate);
        assert_eq!(from_default.audio.channels, from_serde.audio.channels);
        // pairing
        assert_eq!(from_default.pairing.max_attempts, from_serde.pairing.max_attempts);
        assert_eq!(from_default.pairing.lockout_seconds, from_serde.pairing.lockout_seconds);
        // display
        assert_eq!(from_default.display.ipd, from_serde.display.ipd);
        assert_eq!(from_default.display.seconds_from_vsync_to_photons, from_serde.display.seconds_from_vsync_to_photons);
        // foveated
        assert_eq!(from_default.foveated.enabled, from_serde.foveated.enabled);
        assert_eq!(from_default.foveated.fovea_radius, from_serde.foveated.fovea_radius);
        assert_eq!(from_default.foveated.mid_radius, from_serde.foveated.mid_radius);
        assert_eq!(from_default.foveated.mid_qp_offset, from_serde.foveated.mid_qp_offset);
        assert_eq!(from_default.foveated.peripheral_qp_offset, from_serde.foveated.peripheral_qp_offset);
        assert_eq!(from_default.foveated.preset, from_serde.foveated.preset);
        // face_tracking
        assert_eq!(from_default.face_tracking.enabled, from_serde.face_tracking.enabled);
        assert_eq!(from_default.face_tracking.smoothing, from_serde.face_tracking.smoothing);
        assert_eq!(from_default.face_tracking.osc_port, from_serde.face_tracking.osc_port);
        assert_eq!(from_default.face_tracking.active_profile, from_serde.face_tracking.active_profile);
        // sleep_mode
        assert_eq!(from_default.sleep_mode.enabled, from_serde.sleep_mode.enabled);
        assert_eq!(from_default.sleep_mode.timeout_seconds, from_serde.sleep_mode.timeout_seconds);
        assert_eq!(from_default.sleep_mode.motion_threshold, from_serde.sleep_mode.motion_threshold);
        assert_eq!(from_default.sleep_mode.sleep_bitrate_mbps, from_serde.sleep_mode.sleep_bitrate_mbps);
        // memory_monitor
        assert_eq!(from_default.memory_monitor.enabled, from_serde.memory_monitor.enabled);
        assert_eq!(from_default.memory_monitor.poll_interval_seconds, from_serde.memory_monitor.poll_interval_seconds);
        assert_eq!(from_default.memory_monitor.growth_threshold_mb, from_serde.memory_monitor.growth_threshold_mb);
    }
}
