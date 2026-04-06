use std::net::UdpSocket;

/// VRCFaceTracking OSC bridge.
/// Receives HTC facial blendshapes (51 floats) from HMD via TCP,
/// converts to VRChat OSC parameters, and sends to localhost:9000.
pub struct OscBridge {
    socket: Option<UdpSocket>,
    enabled: bool,
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

impl OscBridge {
    pub fn new() -> Self {
        let socket = UdpSocket::bind("0.0.0.0:0").ok();
        if socket.is_some() {
            log::info!("OSC bridge initialized (target: 127.0.0.1:9000)");
        }
        Self {
            socket,
            enabled: true,
        }
    }

    /// Send face data as OSC messages to VRChat (port 9000).
    /// lip: 37 floats, eye: 14 floats.
    pub fn send_face_data(
        &self,
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

        if lip_valid {
            for (i, &value) in lip.iter().enumerate() {
                if value > 0.01 { // Skip near-zero to reduce OSC spam
                    let addr = format!("/avatar/parameters/{}", LIP_NAMES[i]);
                    if let Some(msg) = encode_osc_float(&addr, value) {
                        let _ = socket.send_to(&msg, target);
                    }
                }
            }
        }

        if eye_valid {
            for (i, &value) in eye.iter().enumerate() {
                if value > 0.01 {
                    let addr = format!("/avatar/parameters/{}", EYE_NAMES[i]);
                    if let Some(msg) = encode_osc_float(&addr, value) {
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
}
