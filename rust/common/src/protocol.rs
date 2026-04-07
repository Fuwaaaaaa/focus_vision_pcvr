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

/// Focus Vision PCVR custom header (10 bytes)
/// shard_index/count are u16 to support large IDR keyframes (>256 shards at 80Mbps).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct FvpHeader {
    pub frame_index: u32,
    pub shard_index: u16,
    pub shard_count: u16,
    pub flags: u16,
}

/// Tracking data sent from HMD to PC
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackingData {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
    pub timestamp_ns: u64,
    /// Eye gaze normalized coords (0-1). x=0.5,y=0.5 means center.
    /// gaze_valid=0 means no eye tracking data (use center fallback).
    pub gaze_x: f32,
    pub gaze_y: f32,
    pub gaze_valid: u8,
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
    pub const FACE_DATA: u8 = 0x35;
    pub const HAPTIC_EVENT: u8 = 0x38;
    pub const SLEEP_ENTER: u8 = 0x50;
    pub const SLEEP_EXIT: u8 = 0x51;
    pub const AUDIO_CONFIG: u8 = 0x40;
    pub const AUDIO_START: u8 = 0x41;
    pub const DISCONNECT: u8 = 0xFF;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum VideoCodec {
    H264,
    #[default]
    H265,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtp_header_size_is_12_bytes() {
        assert_eq!(std::mem::size_of::<RtpHeader>(), 12);
    }

    #[test]
    fn fvp_header_size_is_10_bytes() {
        assert_eq!(std::mem::size_of::<FvpHeader>(), 10);
    }

    #[test]
    fn button_flags_no_bit_collisions() {
        let all_flags = [
            buttons::A_X_PRESSED, buttons::B_Y_PRESSED, buttons::MENU_PRESSED,
            buttons::SYSTEM_PRESSED, buttons::THUMBSTICK_CLICK, buttons::TRIGGER_TOUCH,
            buttons::THUMBSTICK_TOUCH, buttons::GRIP_TOUCH,
        ];
        for i in 0..all_flags.len() {
            for j in (i + 1)..all_flags.len() {
                assert_eq!(all_flags[i] & all_flags[j], 0,
                    "Bit collision between flag {} and {}", i, j);
            }
        }
    }

    #[test]
    fn tracking_data_default_has_center_gaze() {
        let td = TrackingData::default();
        assert_eq!(td.gaze_x, 0.0);
        assert_eq!(td.gaze_y, 0.0);
        assert_eq!(td.gaze_valid, 0);
    }

    #[test]
    fn video_codec_default_is_h265() {
        assert_eq!(VideoCodec::default(), VideoCodec::H265);
    }

    #[test]
    fn msg_types_are_unique() {
        let types = [
            msg_type::HELLO, msg_type::HELLO_ACK, msg_type::PIN_REQUEST,
            msg_type::PIN_RESPONSE, msg_type::PIN_RESULT, msg_type::STREAM_CONFIG,
            msg_type::STREAM_START, msg_type::HEARTBEAT, msg_type::HEARTBEAT_ACK,
            msg_type::TRACKING_DATA, msg_type::CONTROLLER_DATA, msg_type::IDR_REQUEST,
            msg_type::FACE_DATA, msg_type::AUDIO_CONFIG, msg_type::AUDIO_START,
            msg_type::DISCONNECT,
        ];
        for i in 0..types.len() {
            for j in (i + 1)..types.len() {
                assert_ne!(types[i], types[j],
                    "Duplicate msg_type at index {} and {}", i, j);
            }
        }
    }
}
