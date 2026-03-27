use serde::{Deserialize, Serialize};

/// RTP header (12 bytes, RFC 3550)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct RtpHeader {
    pub vpxcc: u8,
    pub mpt: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

/// Focus Vision PCVR custom header (8 bytes)
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct FvpHeader {
    pub frame_index: u32,
    pub shard_index: u8,
    pub shard_count: u8,
    pub flags: u16,
}

/// Tracking data sent from HMD to PC
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackingData {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub timestamp_ns: u64,
}

/// Controller state sent from HMD to PC
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ControllerState {
    pub controller_id: u8,
    pub timestamp_ns: u64,
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub trigger: f32,
    pub grip: f32,
    pub thumbstick_x: f32,
    pub thumbstick_y: f32,
    pub button_flags: u32,
    pub battery_level: u8,
}

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
    pub const IDR_REQUEST: u8 = 0x30;
    pub const AUDIO_CONFIG: u8 = 0x40;
    pub const AUDIO_START: u8 = 0x41;
    pub const DISCONNECT: u8 = 0xFF;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoCodec {
    H264,
    H265,
}

impl Default for VideoCodec {
    fn default() -> Self { Self::H265 }
}
