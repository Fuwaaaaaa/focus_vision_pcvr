use std::net::UdpSocket;

/// Default EMA smoothing factor (0.0 = no smoothing, 1.0 = frozen).
/// 0.6 gives a good balance between responsiveness and jitter reduction.
const DEFAULT_SMOOTHING: f32 = 0.6;

/// Default VRChat OSC listener address. The bridge sends here unless
/// overridden via `set_target()` (used by integration tests to bind a
/// loopback receiver on an ephemeral port).
const DEFAULT_TARGET: &str = "127.0.0.1:9000";

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
    /// What each parameter was last sent as (0.0: never, or back to rest).
    sent_lip: [f32; 37],
    sent_eye: [f32; 14],
    /// Active expression profile weights. None = all weights 1.0 (no scaling).
    profile_weights: Option<Vec<f32>>,
    /// Active profile's resting values, taken off before the weights.
    /// None = all 0.0.
    profile_offsets: Option<Vec<f32>>,
    /// OSC destination. `DEFAULT_TARGET` in production; integration tests
    /// override this to route to a captured-loopback receiver.
    target: String,
}

// HTC lip expression names (37), in order of the XrLipExpressionHTC enum
// (openxr.h: XR_LIP_EXPRESSION_JAW_RIGHT_HTC = 0 ... TONGUE_DOWNLEFT_MORPH
// = 36), which is the order the headset sends them in.
// REGRESSION: the tongue (26..36) was in another order, so e.g.
// TONGUE_LEFT drove TongueLongStep2.
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
    "TongueLongStep1", "TongueLeft", "TongueRight", "TongueUp", "TongueDown",
    "TongueRoll", "TongueLongStep2",
    "TongueUpRightMorph", "TongueUpLeftMorph",
    "TongueDownRightMorph", "TongueDownLeftMorph",
];

// HTC eye expression names (14), in order of the XrEyeExpressionHTC enum
// (openxr.h). "Out" is towards that eye's side: the left eye looking out
// looks left, the right eye looking in looks left too.
// REGRESSION: the table was grouped per eye, so e.g. index 2 (RIGHT_BLINK)
// went out as EyeLeftRight.
const EYE_NAMES: [&str; 14] = [
    "EyeLeftBlink", "EyeLeftWide",
    "EyeRightBlink", "EyeRightWide",
    "EyeLeftSqueeze", "EyeRightSqueeze",
    "EyeLeftDown", "EyeRightDown",
    "EyeLeftLeft", "EyeRightLeft", // LEFT_OUT, RIGHT_IN
    "EyeLeftRight", "EyeRightRight", // LEFT_IN, RIGHT_OUT
    "EyeLeftUp", "EyeRightUp",
];

/// Below this a parameter counts as at rest: it is sent as 0.0 once, then
/// not again until it rises (VRChat keeps a parameter's last value).
const REST_THRESHOLD: f32 = 0.01;

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
            log::info!("OSC bridge initialized (target: {}, smoothing={:.2})", DEFAULT_TARGET, smoothing);
        }
        Self {
            socket,
            enabled: true,
            smoothing: smoothing.clamp(0.0, 0.99),
            prev_lip: [0.0; 37],
            prev_eye: [0.0; 14],
            sent_lip: [0.0; 37],
            sent_eye: [0.0; 14],
            profile_weights: None,
            profile_offsets: None,
            target: DEFAULT_TARGET.to_string(),
        }
    }

    /// Override the OSC destination. Used by integration tests to route
    /// to a captured-loopback receiver; production code keeps the
    /// default (`127.0.0.1:9000`, where VRChat listens). Accepts any
    /// `ToSocketAddrs` shape so callers can pass `"127.0.0.1:N"` or a
    /// pre-resolved `SocketAddr`.
    pub fn set_target(&mut self, target: impl Into<String>) {
        self.target = target.into();
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

        let target = self.target.as_str();
        let alpha = self.smoothing;
        let profile = Profile {
            weights: self.profile_weights.as_deref(),
            offsets: self.profile_offsets.as_deref(),
        };

        if lip_valid {
            let group = Group { names: &LIP_NAMES, prev: &mut self.prev_lip, sent: &mut self.sent_lip, first: 0 };
            Self::apply_smoothing_and_send(socket, target, lip, group, alpha, profile);
        }

        if eye_valid {
            let group = Group { names: &EYE_NAMES, prev: &mut self.prev_eye, sent: &mut self.sent_eye, first: 37 };
            Self::apply_smoothing_and_send(socket, target, eye, group, alpha, profile);
        }
    }

    /// EMA smoothing + profile weighting + OSC send for one blendshape group.
    /// A parameter is sent while it is above `REST_THRESHOLD`, and once more
    /// as 0.0 when it falls below. Non-finite inputs are dropped to avoid
    /// poisoning EMA state; the rest are clamped to 0..1.
    fn apply_smoothing_and_send(
        socket: &UdpSocket,
        target: &str,
        raw: &[f32],
        group: Group<'_>,
        alpha: f32,
        profile: Profile<'_>,
    ) {
        for (i, &r) in raw.iter().enumerate() {
            // REGRESSION: only NaN was dropped; +Inf went into the EMA and
            // stayed there.
            if !r.is_finite() {
                continue;
            }
            let smoothed = alpha * group.prev[i] + (1.0 - alpha) * r.clamp(0.0, 1.0);
            group.prev[i] = smoothed;
            let scaled = profile.apply(group.first + i, smoothed);
            // REGRESSION: a parameter falling below the threshold was not
            // sent at all, so with no smoothing (a jump to 0) the avatar kept
            // its last value — a mouth left open.
            let value = if scaled > REST_THRESHOLD { scaled } else { 0.0 };
            if value == 0.0 && group.sent[i] == 0.0 {
                continue;
            }
            group.sent[i] = value;
            let addr = format!("/avatar/parameters/{}", group.names[i]);
            if let Some(msg) = encode_osc_float(&addr, value) {
                match socket.send_to(&msg, target) {
                    Ok(n) => log::trace!("OSC send {} -> {} ({}B sent)", addr, target, n),
                    Err(e) => log::warn!("OSC send to {} failed: {}", target, e),
                }
            }
        }
    }

    /// Get the current smoothed lip values (for testing).
    #[cfg(test)]
    pub fn prev_lip(&self) -> &[f32; 37] {
        &self.prev_lip
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Set the active expression profile (weights and resting values).
    /// None = no scaling (all weights 1.0, offsets 0.0).
    pub fn set_profile(&mut self, profile: Option<&crate::face_tracking::profiles::FtProfile>) {
        self.profile_weights = profile.map(|p| p.weights.clone());
        self.profile_offsets = profile.map(|p| p.offsets.clone());
        if let Some(p) = profile {
            log::info!("FT profile activated: {}", p.name);
            // Apply smoothing override if present
            if let Some(s) = p.smoothing_override {
                self.smoothing = s.clamp(0.0, 0.99);
            }
        } else {
            log::info!("FT profile cleared (using defaults)");
        }
    }
}

/// One blendshape group's names and state; `first` is its index in a
/// profile's 51 values (lip 0, eye 37).
struct Group<'a> {
    names: &'a [&'a str],
    prev: &'a mut [f32],
    sent: &'a mut [f32],
    first: usize,
}

/// The active profile's weights and resting values, if any.
#[derive(Clone, Copy)]
struct Profile<'a> {
    weights: Option<&'a [f32]>,
    offsets: Option<&'a [f32]>,
}

impl Profile<'_> {
    /// (value − resting value) × weight, clamped to 0..1, for blendshape
    /// `index` (0..51).
    fn apply(&self, index: usize, value: f32) -> f32 {
        let weight = self.weights.and_then(|w| w.get(index).copied()).unwrap_or(1.0);
        let offset = self.offsets.and_then(|o| o.get(index).copied()).unwrap_or(0.0);
        ((value - offset) * weight).clamp(0.0, 1.0)
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

    /// `XR_LIP_EXPRESSION_<name>_HTC` in enum order, copied from openxr.h.
    const LIP_ENUM: [&str; 37] = [
        "JAW_RIGHT", "JAW_LEFT", "JAW_FORWARD", "JAW_OPEN", "MOUTH_APE_SHAPE",
        "MOUTH_UPPER_RIGHT", "MOUTH_UPPER_LEFT", "MOUTH_LOWER_RIGHT", "MOUTH_LOWER_LEFT",
        "MOUTH_UPPER_OVERTURN", "MOUTH_LOWER_OVERTURN", "MOUTH_POUT",
        "MOUTH_RAISER_RIGHT", "MOUTH_RAISER_LEFT", "MOUTH_STRETCHER_RIGHT", "MOUTH_STRETCHER_LEFT",
        "CHEEK_PUFF_RIGHT", "CHEEK_PUFF_LEFT", "CHEEK_SUCK",
        "MOUTH_UPPER_UPRIGHT", "MOUTH_UPPER_UPLEFT", "MOUTH_LOWER_DOWNRIGHT", "MOUTH_LOWER_DOWNLEFT",
        "MOUTH_UPPER_INSIDE", "MOUTH_LOWER_INSIDE", "MOUTH_LOWER_OVERLAY",
        "TONGUE_LONGSTEP1", "TONGUE_LEFT", "TONGUE_RIGHT", "TONGUE_UP", "TONGUE_DOWN", "TONGUE_ROLL",
        "TONGUE_LONGSTEP2", "TONGUE_UPRIGHT_MORPH", "TONGUE_UPLEFT_MORPH",
        "TONGUE_DOWNRIGHT_MORPH", "TONGUE_DOWNLEFT_MORPH",
    ];

    /// `XR_EYE_EXPRESSION_<name>_HTC` in enum order, copied from openxr.h.
    const EYE_ENUM: [&str; 14] = [
        "LEFT_BLINK", "LEFT_WIDE", "RIGHT_BLINK", "RIGHT_WIDE", "LEFT_SQUEEZE", "RIGHT_SQUEEZE",
        "LEFT_DOWN", "RIGHT_DOWN", "LEFT_OUT", "RIGHT_IN", "LEFT_IN", "RIGHT_OUT", "LEFT_UP", "RIGHT_UP",
    ];

    fn camel(word: &str) -> String {
        match word {
            // The names this bridge has always sent (SRanipal's).
            "RAISER" => "Smile".into(),
            "STRETCHER" => "Sad".into(),
            "UPRIGHT" => "UpRight".into(),
            "UPLEFT" => "UpLeft".into(),
            "DOWNRIGHT" => "DownRight".into(),
            "DOWNLEFT" => "DownLeft".into(),
            "LONGSTEP1" => "LongStep1".into(),
            "LONGSTEP2" => "LongStep2".into(),
            _ => word[..1].to_string() + &word[1..].to_lowercase(),
        }
    }

    #[test]
    fn lip_names_follow_the_openxr_enum() {
        for (i, e) in LIP_ENUM.iter().enumerate() {
            let expected: String = e.split('_').map(camel).collect();
            assert_eq!(LIP_NAMES[i], expected, "lip {i} ({e})");
        }
    }

    #[test]
    fn eye_names_follow_the_openxr_enum() {
        for (i, e) in EYE_ENUM.iter().enumerate() {
            let (side, what) = e.split_once('_').unwrap();
            // Out is towards the eye's own side, in towards the other.
            let what = match (side, what) {
                ("LEFT", "OUT") | ("RIGHT", "IN") => "Left".to_string(),
                ("LEFT", "IN") | ("RIGHT", "OUT") => "Right".to_string(),
                _ => camel(what),
            };
            assert_eq!(EYE_NAMES[i], format!("Eye{}{what}", camel(side)), "eye {i} ({e})");
        }
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

    #[test]
    fn test_send_face_data_with_profile_weights() {
        let mut bridge = OscBridge::with_smoothing(0.0); // no smoothing for predictable output
        let profile = crate::face_tracking::profiles::FtProfile {
            name: "test".to_string(),
            weights: {
                let mut w = vec![1.0; 51];
                w[3] = 2.0; // JawOpen weight = 2x
                w
            },
            smoothing_override: None,
            offsets: Vec::new(),
        };
        bridge.set_profile(Some(&profile));

        let mut lip = [0.0f32; 37];
        lip[3] = 0.4; // JawOpen raw = 0.4
        let eye = [0.0f32; 14];
        bridge.send_face_data(true, false, &lip, &eye);

        // smoothed = 0.0 * 0.0 + 1.0 * 0.4 = 0.4 (no smoothing)
        assert!((bridge.prev_lip[3] - 0.4).abs() < 1e-6);
        // The scaled value sent would be 0.4 * 2.0 = 0.8 (clamped to [0,1])
    }

    #[test]
    fn test_send_face_data_nan_blendshapes() {
        // NaN in input should NOT poison the EMA state (bug fix: NaN guard)
        let mut bridge = OscBridge::with_smoothing(0.5);
        let eye = [0.0f32; 14];

        // First: set a valid value
        let mut lip = [0.0f32; 37];
        lip[3] = 0.8;
        bridge.send_face_data(true, false, &lip, &eye);
        let prev_after_valid = bridge.prev_lip[3];
        assert!(!prev_after_valid.is_nan());

        // Then: send NaN — should be skipped, prev_lip stays valid
        lip[3] = f32::NAN;
        bridge.send_face_data(true, false, &lip, &eye);
        assert!(!bridge.prev_lip[3].is_nan(), "NaN should not poison EMA state");
        assert!((bridge.prev_lip[3] - prev_after_valid).abs() < 1e-6);
    }

    #[test]
    fn test_send_face_data_profile_clamps_over_1() {
        let mut bridge = OscBridge::with_smoothing(0.0);
        let profile = crate::face_tracking::profiles::FtProfile {
            name: "high-weight".to_string(),
            weights: {
                let mut w = vec![1.0; 51];
                w[0] = 3.0; // 3x weight
                w
            },
            smoothing_override: None,
            offsets: Vec::new(),
        };
        bridge.set_profile(Some(&profile));

        let mut lip = [0.0f32; 37];
        lip[0] = 0.5; // 0.5 * 3.0 = 1.5, should be clamped to 1.0
        let eye = [0.0f32; 14];
        bridge.send_face_data(true, false, &lip, &eye);
        // prev_lip stores the smoothed (unscaled) value
        assert!((bridge.prev_lip[0] - 0.5).abs() < 1e-6);
        // The actual sent value would be clamped, but we can't observe UDP.
        // This test verifies no panic from weight * value > 1.0.
    }

    #[test]
    fn test_smoothing_override_from_profile() {
        let mut bridge = OscBridge::with_smoothing(0.6);
        assert!((bridge.smoothing - 0.6).abs() < 1e-6);

        let profile = crate::face_tracking::profiles::FtProfile {
            name: "smooth".to_string(),
            weights: vec![1.0; 51],
            smoothing_override: Some(0.3),
            offsets: Vec::new(),
        };
        bridge.set_profile(Some(&profile));
        assert!((bridge.smoothing - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_osc_float_encoding_matches_spec() {
        let msg = encode_osc_float("/avatar/parameters/JawOpen", 0.5).unwrap();
        // Address: "/avatar/parameters/JawOpen\0" = 27 bytes → padded to 28
        assert_eq!(&msg[0..26], b"/avatar/parameters/JawOpen");
        assert_eq!(msg[26], 0); // null terminator
        assert_eq!(msg[27], 0); // padding to 28 bytes (4-byte boundary)
        // Type tag at offset 28: ",f\0\0"
        assert_eq!(&msg[28..32], b",f\0\0");
        // Float at offset 32: 0.5 in big-endian
        let float_bytes: [u8; 4] = msg[32..36].try_into().unwrap();
        let value = f32::from_be_bytes(float_bytes);
        assert!((value - 0.5).abs() < 1e-6);
        // Total length: 28 + 4 + 4 = 36
        assert_eq!(msg.len(), 36);
    }

    #[test]
    fn test_parse_face_data_extra_bytes_ignored() {
        // Payload longer than 206 bytes should still parse fine
        let mut payload = vec![0u8; 300];
        payload[0] = 1; // lip_valid
        payload[1] = 1; // eye_valid
        let jaw_off = 2 + 3 * 4;
        payload[jaw_off..jaw_off + 4].copy_from_slice(&0.5f32.to_le_bytes());

        let result = parse_face_data(&payload);
        assert!(result.is_some());
        let (lv, ev, lip, _) = result.unwrap();
        assert!(lv);
        assert!(ev);
        assert!((lip[3] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_eye_and_lip_both_invalid_skips_all() {
        let mut bridge = OscBridge::with_smoothing(0.5);
        let lip = [1.0f32; 37];
        let eye = [1.0f32; 14];
        bridge.send_face_data(false, false, &lip, &eye);
        // Neither lip nor eye should be updated
        assert!((bridge.prev_lip[0] - 0.0).abs() < 1e-6);
        assert!((bridge.prev_eye[0] - 0.0).abs() < 1e-6);
    }
}
