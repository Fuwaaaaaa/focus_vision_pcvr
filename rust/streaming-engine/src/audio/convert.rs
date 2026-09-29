//! Loopback audio to what the Opus encoder takes: interleaved stereo f32 at
//! 48 kHz.
//!
//! WASAPI's shared mode opens a loopback only in the device's mix format
//! (cpal 0.15 checks `IsFormatSupported` and asks for no conversion), so a
//! 44.1 / 96 kHz device or a 5.1 / 7.1 output is converted here: folded to
//! stereo, then resampled with a windowed sinc.

/// The Opus side's sample rate.
pub(crate) const OUTPUT_RATE: u32 = 48_000;

const MINUS_3DB: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// One interleaved frame folded to stereo. Channel order is
/// WAVEFORMATEXTENSIBLE's (FL, FR, FC, LFE, BL, BR, SL, SR): quad, 5.1 and
/// 7.1 add the centre and surrounds to each side at −3 dB (ITU-R BS.775)
/// and leave out the LFE, clamped to ±1. Mono goes to both sides; any other
/// layout keeps its first two channels.
pub fn stereo_frame(frame: &[f32]) -> [f32; 2] {
    let mixed = |l: f32, r: f32| [l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0)];
    match frame.len() {
        0 => [0.0, 0.0],
        1 => [frame[0], frame[0]],
        4 => mixed(frame[0] + MINUS_3DB * frame[2], frame[1] + MINUS_3DB * frame[3]),
        6 => mixed(
            frame[0] + MINUS_3DB * (frame[2] + frame[4]),
            frame[1] + MINUS_3DB * (frame[2] + frame[5]),
        ),
        8 => mixed(
            frame[0] + MINUS_3DB * (frame[2] + frame[4] + frame[6]),
            frame[1] + MINUS_3DB * (frame[2] + frame[5] + frame[7]),
        ),
        _ => [frame[0], frame[1]],
    }
}

/// Zero crossings of the sinc on each side of an output sample.
const ZERO_CROSSINGS: f64 = 16.0;
/// Fractional positions the kernel is tabulated at; between two, it is
/// interpolated.
const PHASES: usize = 256;
/// Passband edge, as a fraction of the lower Nyquist frequency.
const PASSBAND: f64 = 0.95;

/// Stereo sample-rate conversion by a windowed (Blackman) sinc, streaming:
/// chunks can have any length, and feeding a signal in pieces gives the
/// same samples as feeding it whole. Output sample `n` sits at input
/// sample `n · in / out`; it is emitted once the input reaches past its
/// kernel (under 1 ms at 44.1 kHz and up).
pub struct Resampler {
    in_rate: u32,
    out_rate: u32,
    /// Input samples on each side of an output sample the kernel reaches.
    half_taps: usize,
    /// (PHASES + 1) rows of 2 · half_taps weights, each row summing to 1.
    table: Vec<f32>,
    /// Input frames the next outputs still need.
    history: Vec<[f32; 2]>,
    /// The next output's position in `history`: whole samples...
    index: usize,
    /// ...and the fraction, in units of 1 / out_rate.
    frac: u64,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        // Below both Nyquist frequencies: the input's (images when going
        // up) and the output's (aliases when going down).
        let cutoff = (out_rate as f64 / in_rate as f64).min(1.0) * PASSBAND;
        let half_taps = (ZERO_CROSSINGS / cutoff).ceil() as usize;
        let taps = 2 * half_taps;
        let mut table = Vec::with_capacity((PHASES + 1) * taps);
        for p in 0..=PHASES {
            let phase = p as f64 / PHASES as f64;
            let row: Vec<f64> = (0..taps)
                .map(|j| {
                    // Tap j is input sample (index + j + 1 − half_taps).
                    let t = j as f64 + 1.0 - half_taps as f64 - phase;
                    let x = std::f64::consts::PI * cutoff * t;
                    let sinc = if x == 0.0 { 1.0 } else { x.sin() / x };
                    let w = t / half_taps as f64;
                    let blackman = 0.42
                        + 0.5 * (std::f64::consts::PI * w).cos()
                        + 0.08 * (2.0 * std::f64::consts::PI * w).cos();
                    sinc * blackman.max(0.0)
                })
                .collect();
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|&h| (h / sum) as f32));
        }
        Self {
            in_rate,
            out_rate,
            half_taps,
            table,
            // Silence before the first sample, so it can be the first output.
            history: vec![[0.0; 2]; half_taps - 1],
            index: half_taps - 1,
            frac: 0,
        }
    }

    /// Resample `input` (stereo frames), appending interleaved stereo to
    /// `out`.
    pub fn process(&mut self, input: &[[f32; 2]], out: &mut Vec<f32>) {
        if self.in_rate == self.out_rate {
            out.extend(input.iter().flatten());
            return;
        }
        self.history.extend_from_slice(input);
        let taps = 2 * self.half_taps;
        while self.index + self.half_taps < self.history.len() {
            let phase = self.frac as f32 * PHASES as f32 / self.out_rate as f32;
            let p = (phase as usize).min(PHASES - 1);
            let between = phase - p as f32;
            let row0 = &self.table[p * taps..(p + 1) * taps];
            let row1 = &self.table[(p + 1) * taps..(p + 2) * taps];
            let frames = &self.history[self.index + 1 - self.half_taps..self.index + 1 + self.half_taps];
            let mut sample = [0.0f32; 2];
            for ((&h0, &h1), frame) in row0.iter().zip(row1).zip(frames) {
                let h = h0 + (h1 - h0) * between;
                sample[0] += h * frame[0];
                sample[1] += h * frame[1];
            }
            out.extend_from_slice(&sample);

            self.frac += u64::from(self.in_rate);
            self.index += (self.frac / u64::from(self.out_rate)) as usize;
            self.frac %= u64::from(self.out_rate);
        }
        // Keep only what later outputs reach back to.
        let done = (self.index + 1 - self.half_taps).min(self.history.len());
        self.history.drain(..done);
        self.index -= done;
    }
}

/// The capture callback's conversion: the device's interleaved samples
/// (`channels` per frame, at `rate`) to 48 kHz stereo.
pub struct ToOpusFormat {
    channels: usize,
    resampler: Resampler,
    frames: Vec<[f32; 2]>,
}

impl ToOpusFormat {
    pub fn new(rate: u32, channels: u16) -> Self {
        Self {
            channels: usize::from(channels.max(1)),
            resampler: Resampler::new(rate, OUTPUT_RATE),
            frames: Vec::new(),
        }
    }

    /// Convert one callback's samples; a partial frame at the end is dropped
    /// (WASAPI delivers whole frames).
    pub fn convert(&mut self, interleaved: &[f32]) -> Vec<f32> {
        self.frames.clear();
        self.frames.extend(interleaved.chunks(self.channels).filter(|f| f.len() == self.channels).map(stereo_frame));
        let mut out = Vec::with_capacity(self.frames.len() * 2 * OUTPUT_RATE as usize / self.resampler.in_rate as usize + 4);
        self.resampler.process(&self.frames, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn tone(freq: f32, rate: u32, frames: usize) -> Vec<[f32; 2]> {
        (0..frames)
            .map(|n| {
                let v = (2.0 * PI * freq * n as f32 / rate as f32).sin();
                [v, 0.5 * v]
            })
            .collect()
    }

    fn resample(input: &[[f32; 2]], in_rate: u32) -> Vec<f32> {
        let mut r = Resampler::new(in_rate, OUTPUT_RATE);
        let mut out = Vec::new();
        r.process(input, &mut out);
        out
    }

    #[test]
    fn stereo_frames_fold_by_layout() {
        assert_eq!(stereo_frame(&[0.3, -0.2]), [0.3, -0.2]);
        assert_eq!(stereo_frame(&[0.4]), [0.4, 0.4], "mono to both sides");
        // 5.1: FL FR FC LFE BL BR; the LFE (0.9) is left out.
        let [l, r] = stereo_frame(&[0.1, 0.2, 0.3, 0.9, 0.4, 0.5]);
        assert!((l - (0.1 + MINUS_3DB * 0.7)).abs() < 1e-6, "{l}");
        assert!((r - (0.2 + MINUS_3DB * 0.8)).abs() < 1e-6, "{r}");
        // 7.1: side channels join too.
        let [l, r] = stereo_frame(&[0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.2, 0.3]);
        assert!((l - (0.1 + MINUS_3DB * 0.2)).abs() < 1e-6);
        assert!((r - MINUS_3DB * 0.3).abs() < 1e-6);
        // Quad: FL FR BL BR.
        let [l, r] = stereo_frame(&[0.1, 0.2, 0.3, 0.4]);
        assert!((l - (0.1 + MINUS_3DB * 0.3)).abs() < 1e-6);
        assert!((r - (0.2 + MINUS_3DB * 0.4)).abs() < 1e-6);
        assert_eq!(stereo_frame(&[1.0, 1.0, 1.0, 0.0, 1.0, 1.0]), [1.0, 1.0], "clamped");
        assert_eq!(stereo_frame(&[0.1, 0.2, 0.3]), [0.1, 0.2], "unknown layout: front pair");
    }

    #[test]
    fn equal_rates_pass_through() {
        let input = tone(1000.0, OUTPUT_RATE, 100);
        let out = resample(&input, OUTPUT_RATE);
        let expected: Vec<f32> = input.iter().flatten().copied().collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn a_second_in_is_a_second_out() {
        for in_rate in [44_100, 96_000, 192_000, 22_050] {
            let out = resample(&tone(440.0, in_rate, in_rate as usize), in_rate);
            let frames = out.len() / 2;
            // Short of 48 000 only by the samples still waiting for input.
            assert!((47_900..=48_000).contains(&frames), "{in_rate} Hz → {frames} frames");
        }
    }

    #[test]
    fn feeding_in_pieces_gives_the_same_samples() {
        let input = tone(997.0, 44_100, 44_100);
        let whole = resample(&input, 44_100);
        let mut r = Resampler::new(44_100, OUTPUT_RATE);
        let mut pieces = Vec::new();
        let mut rest = &input[..];
        for size in [1, 7, 441, 13, 1024, 3].iter().cycle() {
            if rest.is_empty() {
                break;
            }
            let (chunk, tail) = rest.split_at((*size).min(rest.len()));
            r.process(chunk, &mut pieces);
            rest = tail;
        }
        assert_eq!(pieces, whole);
    }

    /// Largest difference from the ideal tone at 48 kHz, past the start
    /// (where the kernel still reaches into the silence before it).
    fn error_against_ideal(freq: f32, in_rate: u32) -> f32 {
        let out = resample(&tone(freq, in_rate, in_rate as usize / 10), in_rate);
        out.chunks(2)
            .enumerate()
            .skip(200)
            .map(|(n, s)| {
                let ideal = (2.0 * PI * freq * n as f32 / OUTPUT_RATE as f32).sin();
                (s[0] - ideal).abs().max((s[1] - 0.5 * ideal).abs())
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn a_tone_keeps_its_pitch_and_level() {
        // REGRESSION: capture forced 48 kHz, which WASAPI's shared mode
        // refuses on a 44.1 kHz device: no audio at all.
        for (freq, in_rate) in [(1000.0, 44_100), (10_000.0, 44_100), (1000.0, 96_000), (15_000.0, 96_000)] {
            let err = error_against_ideal(freq, in_rate);
            assert!(err < 3e-3, "{freq} Hz from {in_rate} Hz: off by {err}");
        }
    }

    #[test]
    fn going_down_filters_out_what_48khz_cannot_carry() {
        // A 36 kHz tone at 96 kHz would alias to 12 kHz.
        let out = resample(&tone(36_000.0, 96_000, 9_600), 96_000);
        let rms = (out.iter().skip(400).map(|s| s * s).sum::<f32>() / (out.len() - 400) as f32).sqrt();
        assert!(rms < 0.005, "36 kHz leaks through at RMS {rms}");
    }

    #[test]
    fn a_5_1_device_at_44_1khz_becomes_48khz_stereo() {
        let mut convert = ToOpusFormat::new(44_100, 6);
        // 10 ms of 5.1: front left only.
        let mut input = Vec::new();
        for _ in 0..441 {
            input.extend_from_slice(&[0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        }
        let mut out = convert.convert(&input);
        out.extend(convert.convert(&input));
        let frames = out.len() / 2;
        assert!((940..=960).contains(&frames), "{frames} frames for 20 ms");
        let (l, r) = (out[out.len() - 2], out[out.len() - 1]);
        assert!((l - 0.5).abs() < 1e-3 && r.abs() < 1e-6, "({l}, {r})");
    }
}
