use serde::{Deserialize, Serialize};

/// RTP header (12 bytes minimum, RFC 3550)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct RtpHeader {
    /// Version (2), Padding, Extension, CSRC count
    pub vpxcc: u8,
    /// Marker bit + Payload Type
    pub mpt: u8,
    /// Sequence number
    pub sequence: u16,
    /// Timestamp (90kHz clock)
    pub timestamp: u32,
    /// Synchronization source identifier
    pub ssrc: u32,
}

/// Focus Vision PCVR custom header (8 bytes, appended after RTP header)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct FvpHeader {
    /// Frame index (monotonically increasing)
    pub frame_index: u32,
    /// Shard index within FEC group
    pub shard_index: u8,
    /// Total shard count (data + parity)
    pub shard_count: u8,
    /// Flags: bit 0 = keyframe
    pub flags: u16,
}

/// Tracking data sent from HMD to PC
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackingData {
    /// Position in meters (x, y, z)
    pub position: [f32; 3],
    /// Orientation quaternion (x, y, z, w)
    pub orientation: [f32; 4],
    /// Timestamp in nanoseconds (monotonic)
    pub timestamp_ns: u64,
}

/// Controller state sent from HMD to PC
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ControllerState {
    /// 0 = left, 1 = right
    pub controller_id: u8,
    /// Timestamp in nanoseconds
    pub timestamp_ns: u64,
    /// Position (x, y, z)
    pub position: [f32; 3],
    /// Orientation quaternion (x, y, z, w)
    pub orientation: [f32; 4],
    /// Trigger value (0.0 - 1.0)
    pub trigger: f32,
    /// Grip value (0.0 - 1.0)
    pub grip: f32,
    /// Thumbstick X (-1.0 to 1.0)
    pub thumbstick_x: f32,
    /// Thumbstick Y (-1.0 to 1.0)
    pub thumbstick_y: f32,
    /// Button flags (bitfield)
    pub button_flags: u32,
    /// Battery level (0-100)
    pub battery_level: u8,
}

/// Button flag constants
pub mod buttons {
    pub const A_X_PRESSED: u32 = 1 << 0;
    pub const B_Y_PRESSED: u32 = 1 << 1;
    pub const MENU_PRESSED: u32 = 1 << 2;
    pub const SYSTEM_PRESSED: u32 = 1 << 3;
    pub const THUMBSTICK_CLICK: u32 = 1 << 4;
    pub const TRIGGER_TOUCH: u32 = 1 << 5;
    pub const THUMBSTICK_TOUCH: u32 = 1 << 6;
    pub const GRIP_TOUCH: u32 = 1 << 7;
}

/// TCP control channel message types
pub mod msg_type {
    pub const HELLO: u8 = 0x01;
    pub const HELLO_ACK: u8 = 0x02;
    pub const PIN_REQUEST: u8 = 0x03;
    pub const PIN_RESPONSE: u8 = 0x04;
    pub const PIN_RESULT: u8 = 0x05;
    pub const STREAM_CONFIG: u8 = 0x06;
    pub const STREAM_START: u8 = 0x07;
    pub const HEARTBEAT: u8 = 0x10;
    pub const HEARTBEAT_ACK: u8 = 0x11;
    pub const TRACKING_DATA: u8 = 0x20;
    pub const CONTROLLER_DATA: u8 = 0x21;
    pub const DISCONNECT: u8 = 0xFF;
}

/// Video codec selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    H265,
}

impl Default for VideoCodec {
    fn default() -> Self {
        Self::H265
    }
}
