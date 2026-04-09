//! FT Mirror Mode — face tracking camera passthrough from HMD to PC.
//!
//! Receives camera frames from the HMD via TCP (FT_MIRROR_FRAME messages)
//! and buffers them for rendering in the companion app or forwarding to
//! external applications.
//!
//! Hardware required: OpenXR passthrough extension on the HMD.
//! This module implements the PC-side receiver and buffer management.

use std::collections::VecDeque;

/// Camera frame pixel format.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PixelFormat {
    Nv12,       // YUV 4:2:0 semi-planar (most common for camera feeds)
    Jpeg,       // JPEG compressed
    Rgb24,      // Raw RGB 8-bit
}

/// A single camera frame received from the HMD.
#[derive(Debug, Clone)]
pub struct FtMirrorFrame {
    pub width: u16,
    pub height: u16,
    pub format: PixelFormat,
    pub timestamp_us: u64,
    pub data: Vec<u8>,
}

/// Parse an FT_MIRROR_FRAME payload.
/// Format: [width:2B LE][height:2B LE][format:1B][timestamp:8B LE][data...]
pub fn parse_mirror_frame(payload: &[u8]) -> Option<FtMirrorFrame> {
    if payload.len() < 13 { // 2 + 2 + 1 + 8 = 13 bytes header
        return None;
    }
    let width = u16::from_le_bytes([payload[0], payload[1]]);
    let height = u16::from_le_bytes([payload[2], payload[3]]);
    let format = match payload[4] {
        0 => PixelFormat::Nv12,
        1 => PixelFormat::Jpeg,
        2 => PixelFormat::Rgb24,
        _ => return None,
    };
    let timestamp_us = u64::from_le_bytes([
        payload[5], payload[6], payload[7], payload[8],
        payload[9], payload[10], payload[11], payload[12],
    ]);
    let data = payload[13..].to_vec();

    Some(FtMirrorFrame {
        width,
        height,
        format,
        timestamp_us,
        data,
    })
}

/// Encode an FT_MIRROR_REQUEST payload.
/// Format: [enable:1B][max_width:2B LE][max_height:2B LE][preferred_format:1B]
pub fn encode_mirror_request(enable: bool, max_width: u16, max_height: u16, format: PixelFormat) -> Vec<u8> {
    let mut buf = Vec::with_capacity(6);
    buf.push(if enable { 1 } else { 0 });
    buf.extend_from_slice(&max_width.to_le_bytes());
    buf.extend_from_slice(&max_height.to_le_bytes());
    buf.push(match format {
        PixelFormat::Nv12 => 0,
        PixelFormat::Jpeg => 1,
        PixelFormat::Rgb24 => 2,
    });
    buf
}

/// PC-side mirror frame receiver with ring buffer.
/// Drops oldest frames when buffer is full.
pub struct MirrorReceiver {
    buffer: VecDeque<FtMirrorFrame>,
    max_frames: usize,
    frames_received: u64,
    frames_dropped: u64,
    enabled: bool,
}

impl MirrorReceiver {
    pub fn new(max_frames: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(max_frames),
            max_frames,
            frames_received: 0,
            frames_dropped: 0,
            enabled: false,
        }
    }

    /// Enable/disable the mirror receiver.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.buffer.clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Push a received frame into the buffer.
    pub fn push_frame(&mut self, frame: FtMirrorFrame) {
        if !self.enabled {
            return;
        }
        self.frames_received += 1;
        if self.buffer.len() >= self.max_frames {
            self.buffer.pop_front();
            self.frames_dropped += 1;
        }
        self.buffer.push_back(frame);
    }

    /// Get the most recent frame without removing it.
    pub fn latest_frame(&self) -> Option<&FtMirrorFrame> {
        self.buffer.back()
    }

    /// Take the most recent frame, removing it from the buffer.
    pub fn take_latest(&mut self) -> Option<FtMirrorFrame> {
        self.buffer.pop_back()
    }

    pub fn frames_received(&self) -> u64 {
        self.frames_received
    }

    pub fn frames_dropped(&self) -> u64 {
        self.frames_dropped
    }

    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mirror_frame_valid() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&320u16.to_le_bytes()); // width
        payload.extend_from_slice(&240u16.to_le_bytes()); // height
        payload.push(1); // JPEG format
        payload.extend_from_slice(&12345u64.to_le_bytes()); // timestamp
        payload.extend_from_slice(&[0xFF; 100]); // frame data

        let frame = parse_mirror_frame(&payload).unwrap();
        assert_eq!(frame.width, 320);
        assert_eq!(frame.height, 240);
        assert_eq!(frame.format, PixelFormat::Jpeg);
        assert_eq!(frame.timestamp_us, 12345);
        assert_eq!(frame.data.len(), 100);
    }

    #[test]
    fn test_parse_mirror_frame_too_short() {
        assert!(parse_mirror_frame(&[0u8; 12]).is_none()); // < 13 bytes
    }

    #[test]
    fn test_parse_mirror_frame_unknown_format() {
        let mut payload = vec![0u8; 13];
        payload[4] = 99; // unknown format
        assert!(parse_mirror_frame(&payload).is_none());
    }

    #[test]
    fn test_encode_mirror_request() {
        let req = encode_mirror_request(true, 640, 480, PixelFormat::Jpeg);
        assert_eq!(req.len(), 6);
        assert_eq!(req[0], 1); // enabled
        assert_eq!(u16::from_le_bytes([req[1], req[2]]), 640);
        assert_eq!(u16::from_le_bytes([req[3], req[4]]), 480);
        assert_eq!(req[5], 1); // JPEG
    }

    #[test]
    fn test_mirror_receiver_buffer() {
        let mut rx = MirrorReceiver::new(3);
        rx.set_enabled(true);

        for i in 0..5 {
            rx.push_frame(FtMirrorFrame {
                width: 320,
                height: 240,
                format: PixelFormat::Jpeg,
                timestamp_us: i * 1000,
                data: vec![i as u8; 10],
            });
        }

        // Buffer max is 3, so 2 frames dropped
        assert_eq!(rx.buffer_len(), 3);
        assert_eq!(rx.frames_received(), 5);
        assert_eq!(rx.frames_dropped(), 2);

        // Latest should be frame 4 (timestamp 4000)
        let latest = rx.latest_frame().unwrap();
        assert_eq!(latest.timestamp_us, 4000);
    }

    #[test]
    fn test_mirror_receiver_disabled_drops_all() {
        let mut rx = MirrorReceiver::new(10);
        // Disabled by default
        rx.push_frame(FtMirrorFrame {
            width: 320, height: 240, format: PixelFormat::Nv12,
            timestamp_us: 0, data: vec![],
        });
        assert_eq!(rx.buffer_len(), 0);
        assert_eq!(rx.frames_received(), 0);
    }
}
