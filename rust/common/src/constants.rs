/// Default TCP control port
pub const DEFAULT_TCP_PORT: u16 = 9944;
/// Default UDP video/tracking port
pub const DEFAULT_UDP_PORT: u16 = 9945;
/// Maximum Transmission Unit for UDP payloads (conservative for Wi-Fi)
pub const MTU_SIZE: usize = 1400;
/// FEC shard size in bytes
pub const FEC_SHARD_SIZE: usize = 1200;
/// Default FEC redundancy ratio
pub const DEFAULT_FEC_REDUNDANCY: f32 = 0.2;
/// Heartbeat interval in milliseconds
pub const HEARTBEAT_INTERVAL_MS: u64 = 500;
/// Max heartbeat misses before disconnect
pub const HEARTBEAT_MAX_MISSES: u32 = 3;
/// Max PIN pairing attempts before lockout
pub const MAX_PIN_ATTEMPTS: u8 = 3;
/// PIN lockout duration in seconds
pub const PIN_LOCKOUT_SECONDS: u64 = 60;
/// RTP clock rate for video (standard)
pub const RTP_CLOCK_RATE: u32 = 90_000;
/// RTP payload type for H.265
pub const RTP_PT_H265: u8 = 97;
/// RTP payload type for H.264
pub const RTP_PT_H264: u8 = 96;
