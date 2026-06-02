//! Synthetic H.264 / H.265 NAL unit stream for the simulator harness.
//!
//! The headless engine binary (B3) feeds these bytes through
//! `fvp_submit_encoded_nal` so the transport pipeline (RTP packetization,
//! FEC encode/decode, UDP send, slice FEC, recording tap) gets exercised
//! end-to-end without an external encoder dep.
//!
//! What this is NOT: a valid decodable bitstream. The mock-client never
//! decodes the NAL payload — it reassembles frames by index and checks
//! transport-layer invariants. Producing real H.265 syntax would require
//! pulling in libx265 (huge, GPL-licensed) or a baked fixture file that
//! relies on ffmpeg at fixture-generation time. Neither is necessary for
//! the plan's stated goal ("exercise everything downstream of the encoder")
//! and both add CI friction.
//!
//! What this IS: an Annex-B-framed byte stream whose NAL headers carry the
//! correct *type bits* for IDR vs non-IDR frames (so the engine's IDR-flag
//! logic, slice FEC trigger thresholds, and recording-format extension
//! selection all behave identically to a real NVENC stream). Frame sizes
//! follow a realistic distribution (large IDRs, smaller P-frames) so the
//! adaptive bitrate / FEC slicing paths exercise their >16 KB branches.
//!
//! Gated behind the `simulator` feature so production driver builds don't
//! link or expose this module.

use fvp_common::protocol::VideoCodec;

/// 4-byte Annex B start code preceding every NAL unit.
const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// H.265 NAL unit types we emit. Bits 1-6 of the first NAL header byte.
/// Full enum lives in T-REC-H.265 Table 7-1; we only need the keyframe
/// and trailing-picture types.
const H265_NAL_IDR_W_RADL: u8 = 19; // IDR with leading pictures allowed
const H265_NAL_TRAIL_R: u8 = 1; // trailing reference (P-frame analogue)

/// H.264 NAL unit types. Bits 0-4 of the first header byte.
const H264_NAL_IDR: u8 = 5;
const H264_NAL_NON_IDR: u8 = 1;

/// Realistic frame-size targets (bytes). 1832x1920 H.265 at 80 Mbps yields
/// ~890 KB / IDR and ~110 KB / P-frame typical. We scale these down for
/// the simulator so the channel doesn't OOM on a slow CI runner — the
/// transport layer behaves identically at any size above MIN_SLICE_SIZE
/// (16 KB) where slice FEC kicks in.
pub const DEFAULT_IDR_SIZE_BYTES: usize = 24 * 1024; // 24 KB — exceeds slice threshold
pub const DEFAULT_P_FRAME_SIZE_BYTES: usize = 4 * 1024; // 4 KB — below threshold

/// Pre-canned frame index sequence: 1 IDR followed by `gop_size - 1`
/// P-frames, repeating forever.
#[derive(Debug, Clone)]
pub struct SyntheticNalStream {
    codec: VideoCodec,
    gop_size: u32,
    idr_bytes: usize,
    p_bytes: usize,
    /// Monotonically increasing frame index.
    next_frame: u32,
}

impl SyntheticNalStream {
    /// Construct with default frame sizes (IDR 24 KB, P 4 KB) and a 30-frame
    /// GOP (= 3 IDRs/second at 90 fps).
    pub fn new(codec: VideoCodec, gop_size: u32) -> Self {
        Self {
            codec,
            gop_size: gop_size.max(1),
            idr_bytes: DEFAULT_IDR_SIZE_BYTES,
            p_bytes: DEFAULT_P_FRAME_SIZE_BYTES,
            next_frame: 0,
        }
    }

    /// Override the per-frame size targets. Useful when a test wants to
    /// force slice FEC on or off (slice FEC engages for frames >= 16 KB).
    pub fn with_sizes(mut self, idr_bytes: usize, p_bytes: usize) -> Self {
        self.idr_bytes = idr_bytes;
        self.p_bytes = p_bytes;
        self
    }

    /// Scale the default frame sizes in proportion to the encoded pixel area for
    /// a given `render` size and `resolution_scale`. A half-resolution encode
    /// (a quarter of the area) yields quarter-size frames, modelling the
    /// bandwidth a real encoder saves — so the headless E2E can measure the
    /// reduction. `scale == 1.0` leaves the defaults untouched. Uses the same
    /// `compute_encoded_dims` the engine and STREAM_CONFIG use.
    pub fn with_resolution(mut self, render_w: u32, render_h: u32, scale: f32) -> Self {
        let (enc_w, enc_h) = crate::config::compute_encoded_dims(render_w, render_h, scale, 2);
        let encoded_area = enc_w as u64 * enc_h as u64;
        let render_area = (render_w as u64 * render_h as u64).max(1);
        // .max(1): never feed the packetizer a zero-byte frame at extreme scales.
        self.idr_bytes = ((DEFAULT_IDR_SIZE_BYTES as u64 * encoded_area / render_area) as usize).max(1);
        self.p_bytes = ((DEFAULT_P_FRAME_SIZE_BYTES as u64 * encoded_area / render_area) as usize).max(1);
        self
    }

    /// Reset the frame index to 0. Useful for replay tests.
    pub fn reset(&mut self) {
        self.next_frame = 0;
    }

    /// Produce the next frame's full Annex B byte sequence (start code +
    /// NAL header + payload). Returns the bytes plus the metadata needed
    /// by `fvp_submit_encoded_nal`.
    pub fn next_frame(&mut self) -> SyntheticNalFrame {
        let frame_index = self.next_frame;
        let is_idr = frame_index.is_multiple_of(self.gop_size);
        let payload_size = if is_idr { self.idr_bytes } else { self.p_bytes };
        let nal_header_len = match self.codec {
            VideoCodec::H265 => 2,
            VideoCodec::H264 => 1,
        };
        let total = START_CODE.len() + nal_header_len + payload_size;

        let mut bytes = Vec::with_capacity(total);
        bytes.extend_from_slice(&START_CODE);
        match self.codec {
            VideoCodec::H265 => {
                // 16-bit NAL header. Bits 1-6 = nal_unit_type.
                let nal_type = if is_idr { H265_NAL_IDR_W_RADL } else { H265_NAL_TRAIL_R };
                let byte0 = (nal_type & 0x3F) << 1; // forbidden_zero_bit=0, type<<1
                bytes.push(byte0);
                bytes.push(0x01); // nuh_layer_id=0, nuh_temporal_id_plus1=1
            }
            VideoCodec::H264 => {
                // 8-bit NAL header. Bits 0-4 = nal_unit_type.
                let nal_type = if is_idr { H264_NAL_IDR } else { H264_NAL_NON_IDR };
                let nal_ref_idc = if is_idr { 3 } else { 2 };
                bytes.push((nal_ref_idc << 5) | (nal_type & 0x1F));
            }
        }
        // Deterministic payload: byte i = (frame_index XOR i) lower 8 bits.
        // Reproducible across runs, distinguishable across frames, no
        // accidental Annex B start-code collisions (the byte pattern XOR
        // i changes every byte so 00 00 00 01 sequences are extremely
        // unlikely to recur within a single frame).
        let seed = frame_index as u8;
        for i in 0..payload_size {
            bytes.push(seed ^ (i as u8).wrapping_mul(31));
        }

        self.next_frame = self.next_frame.wrapping_add(1);
        SyntheticNalFrame { bytes, frame_index, is_idr }
    }
}

/// One frame as produced by [`SyntheticNalStream::next_frame`].
#[derive(Debug, Clone)]
pub struct SyntheticNalFrame {
    /// Annex B–framed NAL bytes ready for `fvp_submit_encoded_nal`.
    pub bytes: Vec<u8>,
    /// Monotonic frame index that matches the engine's frame_index argument.
    pub frame_index: u32,
    /// `true` if this is an IDR / keyframe.
    pub is_idr: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // These assertions check compile-time constants — clippy flags them as
    // "constant value" because they cannot fail at runtime, but that's
    // exactly the point: the test exists as a compile-time guard rail so
    // a future tweak to DEFAULT_IDR_SIZE_BYTES below 16 KB breaks CI loudly
    // instead of silently skipping the slice-FEC path in scenarios.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_default_idr_size_above_slice_threshold() {
        // Slice FEC engages for frames >= MIN_SLICE_SIZE (16 KB). Default
        // IDR must be above so the simulator exercises the slice path.
        assert!(DEFAULT_IDR_SIZE_BYTES >= 16 * 1024,
            "IDR fixture must exceed slice FEC threshold");
        assert!(DEFAULT_P_FRAME_SIZE_BYTES < 16 * 1024,
            "P-frame fixture must stay below slice FEC threshold to test both paths");
    }

    #[test]
    fn test_with_resolution_scales_frame_bytes_by_area() {
        // Half resolution = a quarter of the encoded area = quarter-size frames,
        // modelling the bandwidth a real encoder saves at resolution_scale=0.5.
        let native = SyntheticNalStream::new(VideoCodec::H265, 60)
            .with_resolution(1832, 1920, 1.0);
        let half = SyntheticNalStream::new(VideoCodec::H265, 60)
            .with_resolution(1832, 1920, 0.5);
        // scale 1.0 leaves the defaults untouched (regression-safe).
        assert_eq!(native.idr_bytes, DEFAULT_IDR_SIZE_BYTES);
        assert_eq!(native.p_bytes, DEFAULT_P_FRAME_SIZE_BYTES);
        // 916*960 / (1832*1920) == 0.25 exactly.
        assert_eq!(half.idr_bytes, DEFAULT_IDR_SIZE_BYTES / 4);
        assert_eq!(half.p_bytes, DEFAULT_P_FRAME_SIZE_BYTES / 4);
    }

    #[test]
    fn test_h265_first_frame_is_idr() {
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 30);
        let f = s.next_frame();
        assert!(f.is_idr, "frame 0 must be IDR");
        assert_eq!(f.frame_index, 0);
        // Annex B start code preserved
        assert_eq!(&f.bytes[..4], &START_CODE);
        // H.265 NAL header: byte0 = (IDR_W_RADL=19) << 1 = 38
        assert_eq!(f.bytes[4], 38);
    }

    #[test]
    fn test_h265_second_frame_is_p() {
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 30);
        let _ = s.next_frame();
        let f = s.next_frame();
        assert!(!f.is_idr);
        assert_eq!(f.frame_index, 1);
        // H.265 NAL header: byte0 = (TRAIL_R=1) << 1 = 2
        assert_eq!(f.bytes[4], 2);
    }

    #[test]
    fn test_gop_pattern_repeats_at_idr() {
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 5);
        let idr_indices: Vec<u32> = (0..20)
            .filter_map(|_| {
                let f = s.next_frame();
                f.is_idr.then_some(f.frame_index)
            })
            .collect();
        assert_eq!(idr_indices, vec![0, 5, 10, 15]);
    }

    #[test]
    fn test_h264_idr_header_nal_type_5() {
        let mut s = SyntheticNalStream::new(VideoCodec::H264, 30);
        let f = s.next_frame();
        assert!(f.is_idr);
        // H.264 NAL header byte: (nal_ref_idc=3 << 5) | nal_type=5 = 101
        assert_eq!(f.bytes[4], 0b01100101);
    }

    #[test]
    fn test_h264_non_idr_header_nal_type_1() {
        let mut s = SyntheticNalStream::new(VideoCodec::H264, 30);
        let _ = s.next_frame();
        let f = s.next_frame();
        // (nal_ref_idc=2 << 5) | nal_type=1 = 65
        assert_eq!(f.bytes[4], 0b01000001);
    }

    #[test]
    fn test_payload_size_matches_codec_overhead() {
        let mut h265 = SyntheticNalStream::new(VideoCodec::H265, 30);
        let f = h265.next_frame();
        // start code (4) + H.265 header (2) + IDR payload
        assert_eq!(f.bytes.len(), 4 + 2 + DEFAULT_IDR_SIZE_BYTES);

        let mut h264 = SyntheticNalStream::new(VideoCodec::H264, 30);
        let f = h264.next_frame();
        // start code (4) + H.264 header (1) + IDR payload
        assert_eq!(f.bytes.len(), 4 + 1 + DEFAULT_IDR_SIZE_BYTES);
    }

    #[test]
    fn test_payload_differs_per_frame() {
        // Frame index variability ensures no two frames are byte-identical,
        // so the receiver-side depacketizer's frame_index tracking is
        // properly exercised.
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 30);
        let f0 = s.next_frame();
        let _ = s.next_frame(); // skip
        let f2 = s.next_frame();
        assert_ne!(f0.bytes, f2.bytes);
    }

    #[test]
    fn test_with_sizes_overrides_defaults() {
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 30)
            .with_sizes(100, 50);
        let idr = s.next_frame();
        let p = s.next_frame();
        assert_eq!(idr.bytes.len(), 4 + 2 + 100);
        assert_eq!(p.bytes.len(), 4 + 2 + 50);
    }

    #[test]
    fn test_reset_returns_to_frame_zero() {
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 30);
        for _ in 0..10 {
            let _ = s.next_frame();
        }
        s.reset();
        let f = s.next_frame();
        assert_eq!(f.frame_index, 0);
        assert!(f.is_idr);
    }

    #[test]
    fn test_zero_gop_size_clamped() {
        // gop_size = 0 would be a divide-by-zero. Constructor must clamp.
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 0);
        let f = s.next_frame();
        // With gop_size clamped to 1, every frame is an IDR.
        assert!(f.is_idr);
        let f = s.next_frame();
        assert!(f.is_idr);
    }

    #[test]
    fn test_frame_index_wraps_safely() {
        // Internal counter is u32; we don't run 4 billion frames in tests
        // but the wrap_add must not panic on overflow.
        let mut s = SyntheticNalStream::new(VideoCodec::H265, 30);
        s.next_frame = u32::MAX;
        let f = s.next_frame();
        assert_eq!(f.frame_index, u32::MAX);
        // After wrap, next is 0
        let f = s.next_frame();
        assert_eq!(f.frame_index, 0);
    }
}
