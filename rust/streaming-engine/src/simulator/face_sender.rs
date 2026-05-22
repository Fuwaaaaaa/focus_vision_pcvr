//! Synthetic Face Tracking sender (TCP FACE_DATA 0x35, 51 blendshapes).
//!
//! Payload mirrors `face_tracking::osc_bridge::parse_face_data` byte-for-byte:
//! `[lip_valid:1B][eye_valid:1B][lip:37×4B LE][eye:14×4B LE]` = 206 bytes.
//!
//! The scenario runner wires this into MockClient so the engine's
//! `FACE_DATA` handler exercises its full path: validate → smooth → emit
//! OSC. A loopback OSC receiver then captures the result for assertion.

// `pub(crate)` so cbindgen leaves these out of the production C header —
// they're simulator-only and shouldn't appear in the driver-facing API.
pub(crate) const LIP_COUNT: usize = 37;
pub(crate) const EYE_COUNT: usize = 14;
pub(crate) const FACE_DATA_PAYLOAD_LEN: usize = 2 + LIP_COUNT * 4 + EYE_COUNT * 4; // 206

// Named indices into the LIP / EYE blendshape arrays. These mirror
// `osc_bridge::LIP_NAMES` / `EYE_NAMES` and are exposed so scenario tests
// can assert on the right channels without re-deriving the layout.
pub(crate) const LIP_JAW_OPEN: usize = 3;
pub(crate) const LIP_SMILE_RIGHT: usize = 12;
pub(crate) const LIP_SMILE_LEFT: usize = 13;
pub(crate) const LIP_SAD_RIGHT: usize = 14;
pub(crate) const LIP_SAD_LEFT: usize = 15;
pub(crate) const EYE_LEFT_BLINK: usize = 0;
pub(crate) const EYE_RIGHT_BLINK: usize = 6;

/// How the synthetic blendshape vector evolves over time. The named
/// presets (`Blink`, `Talk`, `Smile`, `Frown`) only activate the channels
/// they conceptually drive, so scenario assertions can check that the OSC
/// bridge surfaces the right blendshape on the right address.
#[derive(Debug, Clone, Copy)]
pub enum FaceMode {
    /// All blendshapes at 0.0 — exercises the validity flag path without
    /// generating OSC traffic (the bridge skips values below 0.01).
    Relax,
    /// All blendshapes at 1.0 — every channel will exceed the OSC threshold,
    /// so the bridge emits one OSC message per blendshape per send.
    Exaggerate,
    /// Sinusoidal sweep `0..1` at the configured rate. Useful for verifying
    /// that the bridge handles changing values, not just constants.
    SineSweep { hz: f32 },
    /// Pulses both eye blink channels (`EyeLeftBlink`, `EyeRightBlink`) at
    /// `hz` Hz between 0 and 1. All other channels stay at 0. Models the
    /// natural blink cadence (~0.3 Hz) or accelerated test patterns.
    Blink { hz: f32 },
    /// Sinusoidally modulates `JawOpen` between 0 and 0.7 at `hz` Hz. All
    /// other channels at 0. Approximates the avatar mouth motion produced
    /// by speech.
    Talk { hz: f32 },
    /// Static smile: `MouthSmileRight` and `MouthSmileLeft` held at
    /// `intensity` (clamped to 0..=1). All other channels at 0.
    Smile { intensity: f32 },
    /// Static frown: `MouthSadRight` and `MouthSadLeft` held at
    /// `intensity` (clamped to 0..=1). All other channels at 0.
    Frown { intensity: f32 },
}

/// Produce the blendshape vectors for time `t_ns`. Both validity flags
/// are always true (the engine's handler still updates EMA state when
/// they're false — exercising both branches is left to dedicated tests
/// in `osc_bridge.rs`).
pub fn next_face_sample(
    mode: FaceMode,
    t_ns: u64,
) -> (bool, bool, [f32; LIP_COUNT], [f32; EYE_COUNT]) {
    match mode {
        FaceMode::Relax => (true, true, [0.0; LIP_COUNT], [0.0; EYE_COUNT]),
        FaceMode::Exaggerate => (true, true, [1.0; LIP_COUNT], [1.0; EYE_COUNT]),
        FaceMode::SineSweep { hz } => {
            let v = unit_sine(t_ns, hz);
            (true, true, [v; LIP_COUNT], [v; EYE_COUNT])
        }
        FaceMode::Blink { hz } => {
            let mut eye = [0.0f32; EYE_COUNT];
            let v = unit_sine(t_ns, hz);
            eye[EYE_LEFT_BLINK] = v;
            eye[EYE_RIGHT_BLINK] = v;
            (true, true, [0.0; LIP_COUNT], eye)
        }
        FaceMode::Talk { hz } => {
            let mut lip = [0.0f32; LIP_COUNT];
            lip[LIP_JAW_OPEN] = unit_sine(t_ns, hz) * 0.7;
            (true, true, lip, [0.0; EYE_COUNT])
        }
        FaceMode::Smile { intensity } => {
            let v = intensity.clamp(0.0, 1.0);
            let mut lip = [0.0f32; LIP_COUNT];
            lip[LIP_SMILE_RIGHT] = v;
            lip[LIP_SMILE_LEFT] = v;
            (true, true, lip, [0.0; EYE_COUNT])
        }
        FaceMode::Frown { intensity } => {
            let v = intensity.clamp(0.0, 1.0);
            let mut lip = [0.0f32; LIP_COUNT];
            lip[LIP_SAD_RIGHT] = v;
            lip[LIP_SAD_LEFT] = v;
            (true, true, lip, [0.0; EYE_COUNT])
        }
    }
}

/// Sinusoid mapped to `[0, 1]`. Shared by SineSweep, Blink, Talk.
fn unit_sine(t_ns: u64, hz: f32) -> f32 {
    let secs = t_ns as f64 / 1e9;
    let phase = 2.0 * std::f64::consts::PI * hz as f64 * secs;
    (phase.sin() * 0.5 + 0.5) as f32
}

/// Encode a FACE_DATA payload (just the body — the TCP message wrapper
/// adds the length prefix and `0x35` type byte).
pub fn encode_face_data(
    lip_valid: bool,
    eye_valid: bool,
    lip: &[f32; LIP_COUNT],
    eye: &[f32; EYE_COUNT],
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FACE_DATA_PAYLOAD_LEN);
    buf.push(u8::from(lip_valid));
    buf.push(u8::from(eye_valid));
    for v in lip {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in eye {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::face_tracking::osc_bridge::parse_face_data;

    #[test]
    fn payload_length_is_206_bytes() {
        let lip = [0.5f32; LIP_COUNT];
        let eye = [0.5f32; EYE_COUNT];
        let pkt = encode_face_data(true, true, &lip, &eye);
        assert_eq!(pkt.len(), FACE_DATA_PAYLOAD_LEN);
        assert_eq!(pkt.len(), 206);
    }

    #[test]
    fn validity_flags_round_trip() {
        let lip = [0.0; LIP_COUNT];
        let eye = [0.0; EYE_COUNT];
        let pkt = encode_face_data(false, true, &lip, &eye);
        let (lv, ev, _, _) = parse_face_data(&pkt).unwrap();
        assert!(!lv);
        assert!(ev);
    }

    #[test]
    fn encoded_payload_round_trips_through_parser() {
        let mut lip = [0.0f32; LIP_COUNT];
        let mut eye = [0.0f32; EYE_COUNT];
        lip[3] = 0.8; // JawOpen
        eye[0] = 0.5; // EyeLeftBlink
        let pkt = encode_face_data(true, true, &lip, &eye);
        let (lv, ev, parsed_lip, parsed_eye) = parse_face_data(&pkt).unwrap();
        assert!(lv);
        assert!(ev);
        assert_eq!(parsed_lip, lip);
        assert_eq!(parsed_eye, eye);
    }

    #[test]
    fn relax_emits_zero_blendshapes() {
        let (lv, ev, lip, eye) = next_face_sample(FaceMode::Relax, 0);
        assert!(lv);
        assert!(ev);
        assert!(lip.iter().all(|v| *v == 0.0));
        assert!(eye.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn exaggerate_emits_one_blendshapes() {
        let (_, _, lip, eye) = next_face_sample(FaceMode::Exaggerate, 0);
        assert!(lip.iter().all(|v| *v == 1.0));
        assert!(eye.iter().all(|v| *v == 1.0));
    }

    #[test]
    fn sine_sweep_stays_in_unit_range() {
        let mode = FaceMode::SineSweep { hz: 0.5 };
        for t_us in (0..2_000_000u64).step_by(10_000) {
            let (_, _, lip, _) = next_face_sample(mode, t_us * 1_000);
            for v in lip {
                assert!((0.0..=1.0).contains(&v), "value {} out of range", v);
            }
        }
    }

    #[test]
    fn sine_sweep_is_non_constant() {
        let mode = FaceMode::SineSweep { hz: 1.0 };
        let (_, _, lip0, _) = next_face_sample(mode, 0);
        let (_, _, lip_quarter, _) = next_face_sample(mode, 250_000_000); // quarter period
        assert!((lip0[0] - lip_quarter[0]).abs() > 0.1, "sweep should change value");
    }

    #[test]
    fn blink_only_drives_eye_blink_channels() {
        // Sample at the sine peak (quarter period of 1 Hz = 250 ms).
        let (_, _, lip, eye) = next_face_sample(FaceMode::Blink { hz: 1.0 }, 250_000_000);
        assert!(lip.iter().all(|v| *v == 0.0), "Blink must not drive lip channels");
        for (i, v) in eye.iter().enumerate() {
            if i == EYE_LEFT_BLINK || i == EYE_RIGHT_BLINK {
                assert!(*v > 0.9, "blink channel {} should peak near 1.0, got {}", i, v);
            } else {
                assert_eq!(*v, 0.0, "non-blink eye channel {} must stay at 0", i);
            }
        }
    }

    #[test]
    fn talk_only_drives_jaw_open() {
        // Sine peak at t = quarter period of 2 Hz = 125 ms; max amplitude
        // is 0.7 by design (avatars look unnatural with JawOpen pinned to 1).
        let (_, _, lip, eye) = next_face_sample(FaceMode::Talk { hz: 2.0 }, 125_000_000);
        assert!(eye.iter().all(|v| *v == 0.0), "Talk must not drive eye channels");
        for (i, v) in lip.iter().enumerate() {
            if i == LIP_JAW_OPEN {
                assert!(*v > 0.6 && *v <= 0.7, "JawOpen peak should be ~0.7, got {}", v);
            } else {
                assert_eq!(*v, 0.0, "non-JawOpen lip channel {} must stay at 0", i);
            }
        }
    }

    #[test]
    fn smile_drives_both_smile_channels() {
        let (_, _, lip, eye) = next_face_sample(FaceMode::Smile { intensity: 0.8 }, 0);
        assert!(eye.iter().all(|v| *v == 0.0));
        assert_eq!(lip[LIP_SMILE_RIGHT], 0.8);
        assert_eq!(lip[LIP_SMILE_LEFT], 0.8);
        // Pick any non-smile channel to confirm it's zero.
        assert_eq!(lip[LIP_JAW_OPEN], 0.0);
        assert_eq!(lip[LIP_SAD_RIGHT], 0.0);
    }

    #[test]
    fn frown_drives_both_sad_channels_and_clamps_intensity() {
        // Intensity above 1.0 should clamp.
        let (_, _, lip, eye) = next_face_sample(FaceMode::Frown { intensity: 1.5 }, 0);
        assert!(eye.iter().all(|v| *v == 0.0));
        assert_eq!(lip[LIP_SAD_RIGHT], 1.0);
        assert_eq!(lip[LIP_SAD_LEFT], 1.0);
        assert_eq!(lip[LIP_SMILE_RIGHT], 0.0);
    }
}
