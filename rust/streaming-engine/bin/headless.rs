//! focus-vision-headless — drives the streaming engine without SteamVR / NVENC.
//!
//! What it does: loads the config, instantiates `StreamingEngine` directly
//! (bypassing the C ABI / `fvp_init` path that the C++ OpenVR driver uses),
//! and feeds synthetic H.264/H.265 NAL units at the configured framerate.
//! That exercises the full Rust-side transport pipeline (RTP packetization,
//! adaptive FEC, slice FEC, UDP send, recording tap, reconnect state) end-
//! to-end. The companion mock-client (`focus-vision-mock-client`) closes
//! the loop on the receive side.
//!
//! What it does NOT do: NVENC encode (no real video frames exist). Audio is
//! config-driven: with `[audio] enabled = true` and a `synthetic_source`
//! (`"sine"`, `"silence"`, or `"wav"`) the engine generates Opus and sends it
//! over UDP+3 with no audio hardware present; with `synthetic_source = "off"`
//! (the default) the production WASAPI loopback runs and reports None when no
//! output device exists — fine for headless.
//!
//! Gated behind the `simulator` feature. Production builds neither compile
//! nor expose this binary.
//!
//! Usage:
//!     focus-vision-headless                      # config/default.toml, run forever
//!     focus-vision-headless --config foo.toml
//!     focus-vision-headless --duration 5         # exit after 5 wall-clock seconds
//!     focus-vision-headless --frames 450         # exit after 450 frames submitted

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use streaming_engine::config::AppConfig;
use streaming_engine::engine::{EncodedFrame, StreamingEngine};
use streaming_engine::metrics::latency::FrameTimestamps;
use streaming_engine::video::synthetic_nal::SyntheticNalStream;

/// Parsed CLI arguments. Hand-rolled because `clap` would balloon the
/// simulator crate's compile time for two optional flags.
struct Args {
    config: PathBuf,
    duration: Option<Duration>,
    max_frames: Option<u64>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut config = PathBuf::from("config/default.toml");
        let mut duration = None;
        let mut max_frames = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--config" => {
                    config = args.next()
                        .ok_or_else(|| "--config needs a path".to_string())?
                        .into();
                }
                "--duration" => {
                    let s = args.next()
                        .ok_or_else(|| "--duration needs seconds".to_string())?;
                    let secs = s.parse::<u64>()
                        .map_err(|e| format!("--duration not an integer: {}", e))?;
                    duration = Some(Duration::from_secs(secs));
                }
                "--frames" => {
                    let s = args.next()
                        .ok_or_else(|| "--frames needs a count".to_string())?;
                    let n = s.parse::<u64>()
                        .map_err(|e| format!("--frames not an integer: {}", e))?;
                    max_frames = Some(n);
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {}", other)),
            }
        }
        Ok(Self { config, duration, max_frames })
    }
}

fn print_help() {
    let bin = std::env::args().next().unwrap_or_else(|| "focus-vision-headless".to_string());
    println!(
        "{} v{} — simulator-only headless driver for the streaming engine\n",
        bin, env!("CARGO_PKG_VERSION")
    );
    println!("Usage: {} [--config PATH] [--duration SEC] [--frames N]\n", bin);
    println!("  --config PATH    Config file (default: config/default.toml)");
    println!("  --duration SEC   Exit after wall-clock seconds (default: run forever)");
    println!("  --frames N       Exit after submitting N frames (default: no limit)");
    println!("  -h, --help       Show this help");
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = match Args::parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {}\n", e);
            print_help();
            return ExitCode::from(2);
        }
    };

    log::info!(
        "focus-vision-headless v{} starting (config={:?})",
        env!("CARGO_PKG_VERSION"), args.config
    );

    // Load and validate config. `validate` clamps invalid values rather
    // than rejecting them — same graceful behaviour as production.
    let mut config = match AppConfig::load(&args.config.to_string_lossy()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("failed to load config {:?}: {}", args.config, e);
            return ExitCode::from(1);
        }
    };
    for err in config.validate() {
        log::warn!("{}", err);
    }

    let codec = config.video.codec;
    let framerate = config.video.framerate.max(1);
    let frame_period = Duration::from_secs_f64(1.0 / framerate as f64);
    // 1 IDR per second matches the engine's adaptive-bitrate cadence and
    // covers GOP-boundary edge cases in the receiver.
    let gop_size = framerate;

    log::info!(
        "codec={:?} framerate={} ({:?}/frame) gop={}",
        codec, framerate, frame_period, gop_size
    );

    let engine = match StreamingEngine::new(config) {
        Ok(e) => e,
        Err(e) => {
            log::error!("StreamingEngine::new failed: {}", e);
            return ExitCode::from(1);
        }
    };

    let mut stream = SyntheticNalStream::new(codec, gop_size);
    let start = Instant::now();
    let mut next_tick = start;
    let mut frames_sent: u64 = 0;
    let mut frames_dropped: u64 = 0;
    let mut last_log = start;

    loop {
        if let Some(d) = args.duration {
            if start.elapsed() >= d {
                log::info!("duration {:?} reached, stopping", d);
                break;
            }
        }
        if let Some(n) = args.max_frames {
            if frames_sent >= n {
                log::info!("frames limit {} reached, stopping", n);
                break;
            }
        }

        let synth = stream.next_frame();
        let frame = EncodedFrame {
            frame_index: synth.frame_index,
            nal_data: synth.bytes,
            is_idr: synth.is_idr,
            timestamps: FrameTimestamps::new(synth.frame_index),
        };
        if engine.submit_frame(frame) {
            frames_sent += 1;
        } else {
            // submit_frame returns false only when the inbound channel is
            // full (capacity 32). The engine's encoder task drains it as
            // fast as the network sends, so a sustained backlog means the
            // receiver is gone or congested — drop the frame and continue.
            frames_dropped += 1;
        }

        // Periodic progress log: every 5 wall seconds.
        if last_log.elapsed() >= Duration::from_secs(5) {
            log::info!(
                "frames sent={} dropped={} elapsed={:?}",
                frames_sent, frames_dropped, start.elapsed()
            );
            last_log = Instant::now();
        }

        // Drift-free pacing.
        next_tick += frame_period;
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else {
            // Slipped a frame's worth — resync rather than chase.
            next_tick = now;
        }
    }

    log::info!(
        "shutdown: frames sent={} dropped={} duration={:?}",
        frames_sent, frames_dropped, start.elapsed()
    );
    engine.shutdown();
    ExitCode::SUCCESS
}
