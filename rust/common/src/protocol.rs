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
    pub const CONFIG_UPDATE: u8 = 0x55;
    pub const CONFIG_UPDATE_ACK: u8 = 0x56;
    pub const AUDIO_CONFIG: u8 = 0x40;
    pub const AUDIO_START: u8 = 0x41;
    pub const TRANSPORT_FEEDBACK: u8 = 0x12; // HMD → PC: per-packet receive timestamps (GCC)
    pub const CALIBRATE_START: u8 = 0x60;
    pub const CALIBRATE_STATUS: u8 = 0x61;
    pub const FT_MIRROR_REQUEST: u8 = 0x65;   // PC → HMD: request camera feed
    pub const FT_MIRROR_FRAME: u8 = 0x66;     // HMD → PC: camera frame data
    pub const DISCONNECT: u8 = 0xFF;
}

/// Protocol version. Bumped when new message types are added.
/// v1 = initial release (v1.0-v1.3)
/// v2 = added protocol versioning, latency waterfall, UDP optimization (v2.0)
/// v3 = TRANSPORT_FEEDBACK, FVP flags bit layout (slice/stream fields), adaptive FEC (v2.2)
pub const PROTOCOL_VERSION: u16 = 3;

/// Parse protocol version from HELLO payload. Returns 1 if payload is empty (v1 client).
pub fn parse_hello_version(payload: &[u8]) -> u16 {
    if payload.len() >= 2 {
        u16::from_le_bytes([payload[0], payload[1]])
    } else {
        1 // Legacy client with no version field
    }
}

/// Encode protocol version for HELLO/HELLO_ACK payload.
pub fn encode_version(version: u16) -> [u8; 2] {
    version.to_le_bytes()
}

/// FVP header flags bit layout (u16):
///   bit 0:     keyframe
///   bit 1-4:   slice_index (0-15, for slice-based FEC)
///   bit 5-8:   slice_count (0-15, 0 = single slice / legacy)
///   bit 9-10:  stream_id (0=single, 1=fovea, 2=periphery)
///   bit 11-15: reserved
pub mod fvp_flags {
    pub const KEYFRAME: u16       = 1 << 0;
    pub const SLICE_INDEX_SHIFT: u32 = 1;
    pub const SLICE_INDEX_MASK: u16  = 0b1111 << 1;   // bits 1-4
    pub const SLICE_COUNT_SHIFT: u32 = 5;
    pub const SLICE_COUNT_MASK: u16  = 0b1111 << 5;   // bits 5-8
    pub const STREAM_ID_SHIFT: u32   = 9;
    pub const STREAM_ID_MASK: u16    = 0b11 << 9;     // bits 9-10

    /// Encode flags for a single-slice, single-stream packet (v2 compatible).
    pub fn encode_simple(is_keyframe: bool) -> u16 {
        if is_keyframe { KEYFRAME } else { 0 }
    }

    /// Encode full flags with slice and stream information.
    pub fn encode(is_keyframe: bool, slice_index: u8, slice_count: u8, stream_id: u8) -> u16 {
        let mut flags: u16 = 0;
        if is_keyframe { flags |= KEYFRAME; }
        flags |= ((slice_index as u16) & 0xF) << SLICE_INDEX_SHIFT;
        flags |= ((slice_count as u16) & 0xF) << SLICE_COUNT_SHIFT;
        flags |= ((stream_id as u16) & 0x3) << STREAM_ID_SHIFT;
        flags
    }

    /// Encode flags with backward compatibility gate.
    /// If negotiated protocol version < 3, only use keyframe bit (v2 compat).
    /// v2 clients may misinterpret bits 1-10 as keyframe if set.
    pub fn encode_compat(
        negotiated_version: u16,
        is_keyframe: bool,
        slice_index: u8,
        slice_count: u8,
        stream_id: u8,
    ) -> u16 {
        if negotiated_version >= 3 {
            encode(is_keyframe, slice_index, slice_count, stream_id)
        } else {
            encode_simple(is_keyframe)
        }
    }

    pub fn is_keyframe(flags: u16) -> bool { flags & KEYFRAME != 0 }
    pub fn slice_index(flags: u16) -> u8 { ((flags & SLICE_INDEX_MASK) >> SLICE_INDEX_SHIFT) as u8 }
    pub fn slice_count(flags: u16) -> u8 { ((flags & SLICE_COUNT_MASK) >> SLICE_COUNT_SHIFT) as u8 }
    pub fn stream_id(flags: u16) -> u8 { ((flags & STREAM_ID_MASK) >> STREAM_ID_SHIFT) as u8 }
}

/// Transport feedback: per-packet receive timestamps for delay-based bandwidth estimation.
/// Maximum 256 entries per message.
pub const TRANSPORT_FEEDBACK_MAX_ENTRIES: usize = 256;

/// A single transport feedback entry: sequence number + receive timestamp delta (µs).
#[derive(Debug, Clone, Copy)]
pub struct TransportFeedbackEntry {
    pub sequence: u16,
    pub recv_delta_us: i32,
}

/// Parse TRANSPORT_FEEDBACK payload into entries.
/// Format: [count:u16][entries: (seq:u16 + delta:i32) × count]
pub fn parse_transport_feedback(payload: &[u8]) -> Option<Vec<TransportFeedbackEntry>> {
    if payload.len() < 2 { return None; }
    let count = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    if count > TRANSPORT_FEEDBACK_MAX_ENTRIES { return None; }
    let entry_size = 6; // u16 + i32
    if payload.len() < 2 + count * entry_size { return None; }

    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off = 2 + i * entry_size;
        let sequence = u16::from_le_bytes([payload[off], payload[off + 1]]);
        let recv_delta_us = i32::from_le_bytes([
            payload[off + 2], payload[off + 3], payload[off + 4], payload[off + 5],
        ]);
        entries.push(TransportFeedbackEntry { sequence, recv_delta_us });
    }
    Some(entries)
}

/// Encode transport feedback entries into payload bytes.
pub fn encode_transport_feedback(entries: &[TransportFeedbackEntry]) -> Vec<u8> {
    let count = entries.len().min(TRANSPORT_FEEDBACK_MAX_ENTRIES);
    let mut buf = Vec::with_capacity(2 + count * 6);
    buf.extend_from_slice(&(count as u16).to_le_bytes());
    for entry in &entries[..count] {
        buf.extend_from_slice(&entry.sequence.to_le_bytes());
        buf.extend_from_slice(&entry.recv_delta_us.to_le_bytes());
    }
    buf
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
    fn protocol_version_encode_decode() {
        let encoded = encode_version(PROTOCOL_VERSION);
        let decoded = parse_hello_version(&encoded);
        assert_eq!(decoded, PROTOCOL_VERSION);
    }

    #[test]
    fn protocol_version_empty_payload_is_v1() {
        // Legacy clients send empty HELLO — should be treated as v1
        assert_eq!(parse_hello_version(&[]), 1);
    }

    #[test]
    fn protocol_version_single_byte_is_v1() {
        // Partial payload — also treated as legacy v1
        assert_eq!(parse_hello_version(&[2]), 1);
    }

    #[test]
    fn msg_types_are_unique() {
        let types = [
            msg_type::HELLO, msg_type::HELLO_ACK, msg_type::PIN_REQUEST,
            msg_type::PIN_RESPONSE, msg_type::PIN_RESULT, msg_type::STREAM_CONFIG,
            msg_type::STREAM_START, msg_type::HEARTBEAT, msg_type::HEARTBEAT_ACK,
            msg_type::TRANSPORT_FEEDBACK,
            msg_type::TRACKING_DATA, msg_type::CONTROLLER_DATA, msg_type::IDR_REQUEST,
            msg_type::FACE_DATA, msg_type::HAPTIC_EVENT, msg_type::SLEEP_ENTER,
            msg_type::SLEEP_EXIT, msg_type::CONFIG_UPDATE, msg_type::CONFIG_UPDATE_ACK,
            msg_type::CALIBRATE_START, msg_type::CALIBRATE_STATUS,
            msg_type::AUDIO_CONFIG, msg_type::AUDIO_START, msg_type::DISCONNECT,
        ];
        for i in 0..types.len() {
            for j in (i + 1)..types.len() {
                assert_ne!(types[i], types[j],
                    "Duplicate msg_type at index {} and {}", i, j);
            }
        }
    }

    #[test]
    fn fvp_flags_simple_keyframe() {
        let flags = fvp_flags::encode_simple(true);
        assert!(fvp_flags::is_keyframe(flags));
        assert_eq!(fvp_flags::slice_index(flags), 0);
        assert_eq!(fvp_flags::slice_count(flags), 0);
        assert_eq!(fvp_flags::stream_id(flags), 0);
    }

    #[test]
    fn fvp_flags_simple_non_keyframe() {
        let flags = fvp_flags::encode_simple(false);
        assert!(!fvp_flags::is_keyframe(flags));
    }

    #[test]
    fn fvp_flags_full_roundtrip() {
        let flags = fvp_flags::encode(true, 3, 4, 2);
        assert!(fvp_flags::is_keyframe(flags));
        assert_eq!(fvp_flags::slice_index(flags), 3);
        assert_eq!(fvp_flags::slice_count(flags), 4);
        assert_eq!(fvp_flags::stream_id(flags), 2);
    }

    #[test]
    fn fvp_flags_max_values() {
        let flags = fvp_flags::encode(true, 15, 15, 3);
        assert!(fvp_flags::is_keyframe(flags));
        assert_eq!(fvp_flags::slice_index(flags), 15);
        assert_eq!(fvp_flags::slice_count(flags), 15);
        assert_eq!(fvp_flags::stream_id(flags), 3);
    }

    #[test]
    fn fvp_flags_no_overlap_with_keyframe_bit() {
        // slice_index=0, slice_count=0, stream_id=0 with keyframe
        let kf = fvp_flags::encode(true, 0, 0, 0);
        assert_eq!(kf, 1); // only keyframe bit set

        // slice_index=1 should not set keyframe
        let s1 = fvp_flags::encode(false, 1, 0, 0);
        assert!(!fvp_flags::is_keyframe(s1));
        assert_eq!(fvp_flags::slice_index(s1), 1);
    }

    #[test]
    fn transport_feedback_roundtrip() {
        let entries = vec![
            TransportFeedbackEntry { sequence: 100, recv_delta_us: 500 },
            TransportFeedbackEntry { sequence: 101, recv_delta_us: -200 },
            TransportFeedbackEntry { sequence: 102, recv_delta_us: 0 },
        ];
        let encoded = encode_transport_feedback(&entries);
        let decoded = parse_transport_feedback(&encoded).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].sequence, 100);
        assert_eq!(decoded[0].recv_delta_us, 500);
        assert_eq!(decoded[1].sequence, 101);
        assert_eq!(decoded[1].recv_delta_us, -200);
        assert_eq!(decoded[2].recv_delta_us, 0);
    }

    #[test]
    fn transport_feedback_empty() {
        let encoded = encode_transport_feedback(&[]);
        let decoded = parse_transport_feedback(&encoded).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn transport_feedback_rejects_oversized() {
        // Fake a count of 257 (> max 256)
        let mut buf = vec![0u8; 2];
        buf[0] = 0x01;
        buf[1] = 0x01; // count = 257
        assert!(parse_transport_feedback(&buf).is_none());
    }

    #[test]
    fn transport_feedback_rejects_truncated() {
        // count=2 but only 1 entry worth of data
        let mut buf = 2u16.to_le_bytes().to_vec();
        buf.extend_from_slice(&100u16.to_le_bytes());
        buf.extend_from_slice(&500i32.to_le_bytes());
        // Missing second entry
        assert!(parse_transport_feedback(&buf).is_none());
    }

    #[test]
    fn transport_feedback_rejects_too_short() {
        assert!(parse_transport_feedback(&[]).is_none());
        assert!(parse_transport_feedback(&[0]).is_none());
    }

    #[test]
    fn protocol_version_is_3() {
        assert_eq!(PROTOCOL_VERSION, 3);
    }

    #[test]
    fn fvp_flags_compat_v2_uses_simple() {
        // v2 client: only keyframe bit should be set, even if slice/stream data provided
        let flags = fvp_flags::encode_compat(2, true, 3, 4, 2);
        assert!(fvp_flags::is_keyframe(flags));
        assert_eq!(fvp_flags::slice_index(flags), 0); // v2: no slice info
        assert_eq!(fvp_flags::slice_count(flags), 0);
        assert_eq!(fvp_flags::stream_id(flags), 0);
    }

    #[test]
    fn fvp_flags_compat_v3_uses_full() {
        // v3 client: all fields should be encoded
        let flags = fvp_flags::encode_compat(3, true, 3, 4, 2);
        assert!(fvp_flags::is_keyframe(flags));
        assert_eq!(fvp_flags::slice_index(flags), 3);
        assert_eq!(fvp_flags::slice_count(flags), 4);
        assert_eq!(fvp_flags::stream_id(flags), 2);
    }

    #[test]
    fn fvp_flags_compat_v1_uses_simple() {
        // v1 (legacy): same as v2
        let flags = fvp_flags::encode_compat(1, false, 5, 6, 1);
        assert!(!fvp_flags::is_keyframe(flags));
        assert_eq!(flags, 0); // no bits set
    }
}
