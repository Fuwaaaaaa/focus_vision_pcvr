use std::net::UdpSocket;

/// Default EMA smoothing factor (0.0 = no smoothing, 1.0 = frozen).
/// 0.6 gives a good balance between responsiveness and jitter reduction.
const DEFAULT_SMOOTHING: f32 = 0.6;

/// VRCFaceTracking OSC bridge.
/// Receives HTC facial blendshapes (51 floats) from HMD via TCP,
/// applies EMA smoothing to reduce jitter, converts to VRChat OSC
/// parameters, and sends to localhost:9000.
pub struct OscBridge {
    socket: Option<UdpSocket>,
    enabled: bool,
    smoothing: f32,
    prev_lip: [f32; 37],
    prev_eye: [f32; 14],
}

// HTC lip expression names (37), in order of XrLipExpressionHTC enum
const LIP_NAMES: [&str; 37] = [
    "JawRight", "JawLeft", "JawForward", "JawOpen",
    "MouthApeShape", "MouthUpperRight", "MouthUpperLeft",
    "MouthLowerRight", "MouthLowerLeft",
    "MouthUpperOverturn", "MouthLowerOverturn",
    "MouthPout", "MouthSmileRight", "MouthSmileLeft",
    "MouthSadRight", "MouthSadLeft",
    "CheekPuffRight", "CheekPuffLeft", "CheekSuck",
    "MouthUpperUpRight", "MouthUpperUpLeft",
    "MouthLowerDownRight", "MouthLowerDownLeft",
    "MouthUpperInside", "MouthLowerInside",
    "MouthLowerOverlay",
    "TongueLongStep1", "TongueLongStep2",
    "TongueDown", "TongueUp", "TongueRight", "TongueLeft",
    "TongueRoll", "TongueUpLeftMorph", "TongueUpRightMorph",
    "TongueDownLeftMorph", "TongueDownRightMorph",
];

// HTC eye expression names (14), in order of XrEyeExpressionHTC enum
const EYE_NAMES: [&str; 14] = [
    "EyeLeftBlink", "EyeLeftWide", "EyeLeftRight", "EyeLeftLeft",
    "EyeLeftUp", "EyeLeftDown",
    "EyeRightBlink", "EyeRightWide", "EyeRightRight", "EyeRightLeft",
    "EyeRightUp", "EyeRightDown",
    "EyeLeftSqueeze", "EyeRightSqueeze",
];

impl Default for OscBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl OscBridge {
    pub fn new() -> Self {
        Self::with_smoothing(DEFAULT_SMOOTHING)
    }

    pub fn with_smoothing(smoothing: f32) -> Self {
        let socket = UdpSocket::bind("0.0.0.0:0").ok();
        if socket.is_some() {
            log::info!("OSC bridge initialized (target: 127.0.0.1:9000, smoothing={:.2})", smoothing);
        }
        Self {
            socket,
            enabled: true,
            smoothing: smoothing.clamp(0.0, 0.99),
            prev_lip: [0.0; 37],
            prev_eye: [0.0; 14],
        }
    }

    /// Send face data as OSC messages to VRChat (port 9000).
    /// Applies EMA smoothing: smoothed = α * prev + (1-α) * raw.
    /// lip: 37 floats, eye: 14 floats.
    pub fn send_face_data(
        &mut self,
        lip_valid: bool,
        eye_valid: bool,
        lip: &[f32; 37],
        eye: &[f32; 14],
    ) {
        if !self.enabled {
            return;
        }
        let socket = match &self.socket {
            Some(s) => s,
            None => return,
        };

        let target = "127.0.0.1:9000";
        let alpha = self.smoothing;

        if lip_valid {
            for (i, &raw) in lip.iter().enumerate() {
                let smoothed = alpha * self.prev_lip[i] + (1.0 - alpha) * raw;
                self.prev_lip[i] = smoothed;
                if smoothed > 0.01 {
                    let addr = format!("/avatar/parameters/{}", LIP_NAMES[i]);
                    if let Some(msg) = encode_osc_float(&addr, smoothed) {
                        let _ = socket.send_to(&msg, target);
                    }
                }
            }
        }

        if eye_valid {
            for (i, &raw) in eye.iter().enumerate() {
                let smoothed = alpha * self.prev_eye[i] + (1.0 - alpha) * raw;
                self.prev_eye[i] = smoothed;
                if smoothed > 0.01 {
                    let addr = format!("/avatar/parameters/{}", EYE_NAMES[i]);
                    if let Some(msg) = encode_osc_float(&addr, smoothed) {
                        let _ = socket.send_to(&msg, target);
                    }
                }
            }
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// Parse face data payload from FACE_DATA TCP message.
/// Format: [lip_valid:1B][eye_valid:1B][lip:37×4B][eye:14×4B] = 206 bytes.
/// Returns (lip_valid, eye_valid, lip[37], eye[14]).
pub fn parse_face_data(payload: &[u8]) -> Option<(bool, bool, [f32; 37], [f32; 14])> {
    if payload.len() < 206 {
        return None;
    }
    let lip_valid = payload[0] != 0;
    let eye_valid = payload[1] != 0;

    let mut lip = [0.0f32; 37];
    for (i, val) in lip.iter_mut().enumerate() {
        let off = 2 + i * 4;
        *val = f32::from_le_bytes([
            payload[off], payload[off + 1], payload[off + 2], payload[off + 3],
        ]);
    }

    let mut eye = [0.0f32; 14];
    for (i, val) in eye.iter_mut().enumerate() {
        let off = 2 + 37 * 4 + i * 4;
        *val = f32::from_le_bytes([
            payload[off], payload[off + 1], payload[off + 2], payload[off + 3],
        ]);
    }

    Some((lip_valid, eye_valid, lip, eye))
}

/// Encode a minimal OSC message: address + ",f" type tag + float value.
/// OSC spec: all strings null-terminated and padded to 4-byte boundary.
fn encode_osc_float(address: &str, value: f32) -> Option<Vec<u8>> {
    let mut msg = Vec::with_capacity(64);

    // Address string (null-terminated, padded to 4 bytes)
    msg.extend_from_slice(address.as_bytes());
    msg.push(0);
    while msg.len() % 4 != 0 {
        msg.push(0);
    }

    // Type tag string ",f\0" padded to 4 bytes
    msg.extend_from_slice(b",f\0\0");

    // Float value (big-endian per OSC spec)
    msg.extend_from_slice(&value.to_be_bytes());

    Some(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_osc_float_basic() {
        let msg = encode_osc_float("/test", 1.0).unwrap();
        // Address: "/test\0" padded to 8 bytes, type tag: ",f\0\0" (4 bytes), float: 4 bytes
        assert_eq!(msg.len(), 8 + 4 + 4);
        // Check address starts with /test
        assert_eq!(&msg[0..5], b"/test");
        // Check type tag
        assert_eq!(&msg[8..12], b",f\0\0");
    }

    #[test]
    fn test_encode_osc_address_padding() {
        // "/ab" = 3 chars + null = 4 bytes (already aligned)
        let msg = encode_osc_float("/ab", 0.5).unwrap();
        assert_eq!(msg.len(), 4 + 4 + 4); // addr(4) + tag(4) + float(4)
    }

    #[test]
    fn test_lip_names_count() {
        assert_eq!(LIP_NAMES.len(), 37);
    }

    #[test]
    fn test_eye_names_count() {
        assert_eq!(EYE_NAMES.len(), 14);
    }

    #[test]
    fn test_osc_bridge_creation() {
        let bridge = OscBridge::new();
        assert!(bridge.socket.is_some());
    }

    #[test]
    fn test_osc_float_value_encoding() {
        let msg = encode_osc_float("/x", 0.75).unwrap();
        // Float at the last 4 bytes, big-endian per OSC spec
        let float_bytes = &msg[msg.len() - 4..];
        let value = f32::from_be_bytes(float_bytes.try_into().unwrap());
        assert!((value - 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_send_face_data_skips_near_zero() {
        // Verify that near-zero values don't generate OSC traffic.
        // We can't easily observe UDP, but we test that the code path runs
        // without error. The bridge sends only values > 0.01.
        let mut bridge = OscBridge::new();
        let mut lip = [0.0f32; 37];
        let mut eye = [0.0f32; 14];

        // All zeros — should send nothing
        bridge.send_face_data(true, true, &lip, &eye);

        // Set one lip and one eye above threshold
        lip[3] = 0.8; // JawOpen
        eye[0] = 0.5; // EyeLeftBlink
        bridge.send_face_data(true, true, &lip, &eye);
    }

    #[test]
    fn test_send_face_data_disabled() {
        let mut bridge = OscBridge::new();
        bridge.set_enabled(false);
        let lip = [1.0f32; 37];
        let eye = [1.0f32; 14];
        // Should return immediately without sending
        bridge.send_face_data(true, true, &lip, &eye);
    }

    #[test]
    fn test_all_lip_names_generate_valid_osc() {
        for name in &LIP_NAMES {
            let addr = format!("/avatar/parameters/{}", name);
            let msg = encode_osc_float(&addr, 1.0).unwrap();
            // Address must be null-terminated and 4-byte aligned
            assert_eq!(msg.len() % 4, 0);
            // Must contain the type tag
            let tag_pos = msg.windows(4).position(|w| w == b",f\0\0");
            assert!(tag_pos.is_some(), "Missing type tag for {}", name);
        }
    }

    #[test]
    fn test_all_eye_names_generate_valid_osc() {
        for name in &EYE_NAMES {
            let addr = format!("/avatar/parameters/{}", name);
            let msg = encode_osc_float(&addr, 0.5).unwrap();
            assert_eq!(msg.len() % 4, 0);
            let tag_pos = msg.windows(4).position(|w| w == b",f\0\0");
            assert!(tag_pos.is_some(), "Missing type tag for {}", name);
        }
    }

    #[test]
    fn test_parse_face_data_valid() {
        let mut payload = vec![0u8; 206];
        payload[0] = 1; // lip_valid
        payload[1] = 1; // eye_valid
        // Set JawOpen (index 3) = 0.8
        let jaw_off = 2 + 3 * 4;
        payload[jaw_off..jaw_off + 4].copy_from_slice(&0.8f32.to_le_bytes());
        // Set EyeLeftBlink (index 0) = 0.5
        let eye_off = 2 + 37 * 4;
        payload[eye_off..eye_off + 4].copy_from_slice(&0.5f32.to_le_bytes());

        let (lv, ev, lip, eye) = parse_face_data(&payload).unwrap();
        assert!(lv);
        assert!(ev);
        assert!((lip[3] - 0.8).abs() < 1e-6);
        assert!((eye[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_parse_face_data_too_short() {
        assert!(parse_face_data(&[0u8; 100]).is_none());
        assert!(parse_face_data(&[0u8; 205]).is_none());
    }

    #[test]
    fn test_parse_face_data_validity_flags() {
        let mut payload = vec![0u8; 206];
        payload[0] = 0; // lip not valid
        payload[1] = 1; // eye valid
        let (lv, ev, _, _) = parse_face_data(&payload).unwrap();
        assert!(!lv);
        assert!(ev);
    }

    #[test]
    fn test_ema_smoothing_converges() {
        // With smoothing=0.5, value should converge toward input over multiple frames
        let mut bridge = OscBridge::with_smoothing(0.5);
        let lip = [1.0f32; 37];
        let eye = [0.0f32; 14];

        // Feed same value multiple times — prev_lip should converge toward 1.0
        for _ in 0..10 {
            bridge.send_face_data(true, false, &lip, &eye);
        }
        // After 10 frames with α=0.5, prev should be very close to 1.0
        // Each step: prev = 0.5 * prev + 0.5 * 1.0 → converges to 1.0
        assert!((bridge.prev_lip[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_no_smoothing() {
        let mut bridge = OscBridge::with_smoothing(0.0);
        let lip = [0.75f32; 37];
        let eye = [0.0f32; 14];
        bridge.send_face_data(true, false, &lip, &eye);
        // With α=0.0, output = raw value immediately
        assert!((bridge.prev_lip[0] - 0.75).abs() < 1e-6);
    }
}
