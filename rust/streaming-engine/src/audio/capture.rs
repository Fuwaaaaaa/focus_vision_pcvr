use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use tokio::sync::mpsc;

/// Audio samples captured from WASAPI loopback.
/// Always normalized to f32, interleaved stereo, 48kHz.
pub struct AudioCapture {
    stream: Option<Stream>,
    sample_rate: u32,
    channels: u16,
}

/// Raw audio chunk: variable-length f32 samples from a single callback invocation.
pub type AudioChunk = Vec<f32>;

impl AudioCapture {
    /// Start WASAPI loopback capture on the default output device.
    /// Sends raw audio chunks (variable-length) to the provided channel.
    /// The consumer is responsible for accumulating into fixed-size Opus frames.
    ///
    /// Returns None if no audio output device is available (non-fatal).
    pub fn start(chunk_tx: mpsc::Sender<AudioChunk>) -> Option<Self> {
        let host = cpal::default_host();

        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                log::warn!("No audio output device found — audio streaming disabled");
                return None;
            }
        };

        let device_name = device.name().unwrap_or_else(|_| "unknown".into());
        log::info!("Audio capture: using device '{}'", device_name);

        let config = match device.default_output_config() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Failed to get audio config: {} — audio disabled", e);
                return None;
            }
        };

        let device_sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        // Force 48kHz to match Opus encoder. WASAPI handles resampling internally
        // when the requested rate differs from the device's native rate.
        // This prevents sample rate mismatch (e.g., 44.1kHz device → 48kHz Opus).
        let sample_rate = 48000u32;
        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        if device_sample_rate != sample_rate {
            log::info!(
                "Audio capture: device native {}Hz, requesting {}Hz (WASAPI resample)",
                device_sample_rate, sample_rate
            );
        }

        log::info!(
            "Audio capture config: {}Hz, {} ch, {:?}",
            sample_rate, channels, sample_format
        );

        let err_fn = |err: cpal::StreamError| {
            log::error!("Audio capture stream error: {}", err);
        };

        // Lock-free: callback sends raw chunks directly via try_send (never blocks).
        let ch = channels;
        let tx_f32 = chunk_tx.clone();
        let tx_i16 = chunk_tx;

        let stream_result = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let chunk = if ch == 1 {
                        // Mono → stereo: duplicate each sample
                        let mut stereo = Vec::with_capacity(data.len() * 2);
                        for &s in data {
                            stereo.push(s);
                            stereo.push(s);
                        }
                        stereo
                    } else {
                        data.to_vec()
                    };
                    let _ = tx_f32.try_send(chunk);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let chunk: Vec<f32> = if ch == 1 {
                        let mut stereo = Vec::with_capacity(data.len() * 2);
                        for &s in data {
                            let f = s as f32 / 32768.0;
                            stereo.push(f);
                            stereo.push(f);
                        }
                        stereo
                    } else {
                        data.iter().map(|&s| s as f32 / 32768.0).collect()
                    };
                    let _ = tx_i16.try_send(chunk);
                },
                err_fn,
                None,
            ),
            _ => {
                log::warn!("Unsupported sample format {:?} — audio disabled", sample_format);
                return None;
            }
        };

        let stream = match stream_result {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to build audio stream: {} — audio disabled", e);
                return None;
            }
        };

        if let Err(e) = stream.play() {
            log::warn!("Failed to start audio stream: {} — audio disabled", e);
            return None;
        }

        log::info!("Audio loopback capture started (lock-free)");

        Some(Self {
            stream: Some(stream),
            sample_rate,
            channels,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            drop(stream);
            log::info!("Audio capture stopped");
        }
    }
}
