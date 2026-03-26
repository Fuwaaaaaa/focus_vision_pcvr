use serde::{Deserialize, Serialize};
use fvp_common::protocol::VideoCodec;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub pairing: PairingConfig,
    #[serde(default)]
    pub display: DisplayConfig,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u8,
    #[serde(default = "default_lockout_seconds")]
    pub lockout_seconds: u64,
}

fn default_tcp_port() -> u16 { fvp_common::DEFAULT_TCP_PORT }
fn default_udp_port() -> u16 { fvp_common::DEFAULT_UDP_PORT }
fn default_fec_redundancy() -> f32 { fvp_common::DEFAULT_FEC_REDUNDANCY }
fn default_bitrate() -> u32 { 80 }
fn default_resolution() -> [u32; 2] { [1832, 1920] }
fn default_framerate() -> u32 { 90 }
fn default_ipd() -> f32 { 0.063 }
fn default_vsync_to_photons() -> f32 { 0.011 }
fn default_max_attempts() -> u8 { fvp_common::MAX_PIN_ATTEMPTS }
fn default_lockout_seconds() -> u64 { fvp_common::PIN_LOCKOUT_SECONDS }

impl Default for NetworkConfig {
    fn default() -> Self {
        Self { tcp_port: default_tcp_port(), udp_port: default_udp_port(), fec_redundancy: default_fec_redundancy() }
    }
}
impl Default for VideoConfig {
    fn default() -> Self {
        Self { codec: VideoCodec::default(), bitrate_mbps: default_bitrate(), resolution_per_eye: default_resolution(), framerate: default_framerate() }
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
impl Default for AppConfig {
    fn default() -> Self {
        Self { network: NetworkConfig::default(), video: VideoConfig::default(), pairing: PairingConfig::default(), display: DisplayConfig::default() }
    }
}

impl AppConfig {
    pub fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
}
