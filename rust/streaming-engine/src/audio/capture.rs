use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Audio samples captured from WASAPI loopback.
/// Always normalized to f32, interleaved stereo, 48kHz.
pub struct AudioCapture {
    stream: Option<Stream>,
    sample_rate: u32,
    channels: u16,
}

/// Raw audio frame: interleaved f32 samples for one Opus frame (10ms = 480 samples/ch).
pub type AudioFrame = Vec<f32>;

const OPUS_FRAME_SAMPLES: usize = 480; // 10ms at 48kHz

impl AudioCapture {
    /// Start WASAPI loopback capture on the default output device.
    /// Sends audio frames (10ms, 48kHz, stereo) to the provided channel.
    ///
    /// Returns None if no audio output device is available (non-fatal).
    pub fn start(frame_tx: mpsc::Sender<AudioFrame>) -> Option<Self> {
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

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let sample_format = config.sample_format();

        log::info!(
            "Audio capture config: {}Hz, {} ch, {:?}",
            sample_rate, channels, sample_format
        );

        // Accumulation buffer for collecting samples into 10ms frames
        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(
            OPUS_FRAME_SAMPLES * channels as usize * 2,
        )));

        let buf_clone = buffer.clone();
        let tx = frame_tx.clone();
        let ch = channels;

        // Build the input stream using loopback capture.
        // cpal uses WASAPI loopback when capturing from an output device.
        let err_fn = |err: cpal::StreamError| {
            log::error!("Audio capture stream error: {}", err);
        };

        let stream_result = match sample_format {
            SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    accumulate_and_send(data, &buf_clone, &tx, ch);
                },
                err_fn,
                None,
            ),
            SampleFormat::I16 => {
                let buf_c = buffer.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        // Convert i16 to f32
                        let f32_data: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                        accumulate_and_send(&f32_data, &buf_c, &tx, ch);
                    },
                    err_fn,
                    None,
                )
            }
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

        log::info!("Audio loopback capture started");

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

/// Accumulate incoming samples into fixed-size Opus frames and send them.
fn accumulate_and_send(
    data: &[f32],
    buffer: &Arc<Mutex<Vec<f32>>>,
    tx: &mpsc::Sender<AudioFrame>,
    channels: u16,
) {
    let mut buf = match buffer.lock() {
        Ok(b) => b,
        Err(_) => return,
    };

    // If device is mono, duplicate to stereo
    if channels == 1 {
        for &sample in data {
            buf.push(sample);
            buf.push(sample); // duplicate to right channel
        }
    } else {
        buf.extend_from_slice(data);
    }

    // Extract complete frames
    let stereo_frame_size = OPUS_FRAME_SAMPLES * 2; // stereo
    while buf.len() >= stereo_frame_size {
        let frame: Vec<f32> = buf.drain(..stereo_frame_size).collect();
        let _ = tx.try_send(frame); // Drop if channel full (audio is lossy)
    }
}
