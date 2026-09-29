use audiopus::coder::Encoder;
use audiopus::{Application, Channels, SampleRate, Bitrate};

/// Opus audio encoder for low-latency VR streaming.
///
/// Encodes 10ms frames (480 samples/ch @ 48kHz) of stereo f32 audio
/// into Opus packets. Uses LowDelay mode for minimum latency
/// with Opus in-band FEC for packet loss resilience.
pub struct AudioEncoder {
    encoder: Encoder,
    encode_buf: Vec<u8>,
}

impl AudioEncoder {
    /// Create a new Opus encoder.
    /// `bitrate`: target bitrate in bps (e.g., 128000 for 128kbps).
    pub fn new(bitrate: u32) -> Result<Self, String> {
        let mut encoder = Encoder::new(
            SampleRate::Hz48000,
            Channels::Stereo,
            Application::LowDelay,
        )
        .map_err(|e| format!("Failed to create Opus encoder: {e}"))?;

        let bps = i32::try_from(bitrate).map_err(|_| format!("Bitrate {bitrate} bps out of range"))?;
        encoder
            .set_bitrate(Bitrate::BitsPerSecond(bps))
            .map_err(|e| format!("Failed to set bitrate: {e}"))?;

        encoder
            .set_inband_fec(true)
            .map_err(|e| format!("Failed to enable FEC: {e}"))?;

        encoder
            .set_packet_loss_perc(5)
            .map_err(|e| format!("Failed to set packet loss: {e}"))?;

        log::info!("Opus encoder: 48kHz stereo, {}kbps, FEC enabled", bitrate / 1000);

        Ok(Self {
            encoder,
            encode_buf: vec![0u8; 4000],
        })
    }

    /// Encode one frame of interleaved f32 stereo audio (480 samples/ch = 10ms).
    /// Returns the Opus-encoded packet bytes.
    pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>, String> {
        let pcm_i16: Vec<i16> = pcm
            .iter()
            .map(|&s| {
                let clamped = s.clamp(-1.0, 1.0);
                (clamped * 32767.0) as i16
            })
            .collect();

        let len = self
            .encoder
            .encode(&pcm_i16, &mut self.encode_buf)
            .map_err(|e| format!("Opus encode error: {e}"))?;

        Ok(self.encode_buf[..len].to_vec())
    }

    /// The target bitrate the encoder is set to, in bps.
    pub fn bitrate(&self) -> Option<u32> {
        match self.encoder.bitrate() {
            Ok(Bitrate::BitsPerSecond(bps)) => u32::try_from(bps).ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let encoder = AudioEncoder::new(128_000);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_encode_silence() {
        let mut encoder = AudioEncoder::new(128_000).unwrap();
        let silence = vec![0.0f32; 960]; // 480 samples * 2 channels
        let result = encoder.encode(&silence);
        assert!(result.is_ok());
        let packet = result.unwrap();
        assert!(!packet.is_empty());
    }

    #[test]
    fn test_encode_tone() {
        let mut encoder = AudioEncoder::new(128_000).unwrap();
        let mut pcm = Vec::with_capacity(960);
        for i in 0..480 {
            let sample = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin() * 0.5;
            pcm.push(sample);
            pcm.push(sample);
        }
        let result = encoder.encode(&pcm);
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_encode_clamps_out_of_range() {
        let mut encoder = AudioEncoder::new(128_000).unwrap();
        // Values outside [-1.0, 1.0] should be clamped, not cause errors
        let mut pcm = vec![2.0f32; 960]; // all > 1.0
        pcm[0] = -5.0; // way below -1.0
        let result = encoder.encode(&pcm);
        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_multiple_frames_consecutive() {
        let mut encoder = AudioEncoder::new(128_000).unwrap();
        let silence = vec![0.0f32; 960];
        // Opus encoder is stateful — verify consecutive frames work
        for _ in 0..10 {
            let result = encoder.encode(&silence);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_tone_packet_smaller_than_silence() {
        // Opus compresses silence better than tones
        let mut encoder = AudioEncoder::new(128_000).unwrap();

        let silence = vec![0.0f32; 960];
        let silence_pkt = encoder.encode(&silence).unwrap();

        let mut tone = Vec::with_capacity(960);
        for i in 0..480 {
            let s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48000.0).sin() * 0.8;
            tone.push(s);
            tone.push(s);
        }
        let tone_pkt = encoder.encode(&tone).unwrap();

        // Tone should generally produce a larger packet than silence
        assert!(tone_pkt.len() >= silence_pkt.len(),
            "tone={} bytes, silence={} bytes", tone_pkt.len(), silence_pkt.len());
    }

    /// Bytes for one second (100 frames) of full-band noise at `bitrate`.
    fn bytes_per_second_of_noise(bitrate: u32) -> usize {
        let mut encoder = AudioEncoder::new(bitrate).unwrap();
        assert_eq!(encoder.bitrate(), Some(bitrate));
        let mut state: u32 = 0x1234_5678;
        let mut total = 0;
        for _ in 0..100 {
            let pcm: Vec<f32> = (0..960)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 8) as f32 / (1u32 << 24) as f32 - 0.5
                })
                .collect();
            total += encoder.encode(&pcm).unwrap().len();
        }
        total
    }

    #[test]
    fn test_bitrate_sets_the_packet_size() {
        // REGRESSION: the engine made every encoder 128 kbps, whatever
        // `[audio] bitrate_kbps` said.
        let low = bytes_per_second_of_noise(64_000);
        let high = bytes_per_second_of_noise(256_000);
        assert!((6_000..=10_000).contains(&low), "64 kbps gave {low} B/s");
        assert!((26_000..=36_000).contains(&high), "256 kbps gave {high} B/s");
    }
}
