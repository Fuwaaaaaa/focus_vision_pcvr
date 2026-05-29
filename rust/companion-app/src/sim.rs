//! Integrated simulation mode (feature = "simulator").
//!
//! Runs a real `StreamingEngine` plus a mock HMD client *in-process* on two
//! background threads, so the companion's normal `status.json`-polling UI
//! lights up a genuine Connected session with live stats — no VR headset,
//! SteamVR, NVIDIA GPU, or Android device required. This is the GUI-button
//! promotion of `streaming-engine/tests/headless_e2e_test.rs`, which already
//! proves the in-process engine + mock-client composition.
//!
//! Lifecycle: [`start`] spawns the threads and returns a [`SimHandle`].
//! Dropping the handle (or calling [`SimHandle::stop`]) cancels both threads
//! and joins them deterministically — wired into the app's window-close path
//! so simulation never outlives the window.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use streaming_engine::config::AppConfig;
use streaming_engine::engine::{EncodedFrame, StreamingEngine};
use streaming_engine::metrics::latency::FrameTimestamps;
use streaming_engine::simulator::test_helpers::{delete_stale_status, pick_free_ports, wait_for_pin};
use streaming_engine::simulator::{run as run_mock_client, MockClientConfig};
use streaming_engine::video::synthetic_nal::SyntheticNalStream;
use tokio_util::sync::CancellationToken;

/// Health/diagnostics shared from the worker threads back to the UI thread.
#[derive(Default, Clone)]
pub(crate) struct SimStatus {
    /// First fatal error from either thread; surfaced in the banner. `None` =
    /// healthy.
    pub error: Option<String>,
}

/// Owns every resource a running simulation needs. `Drop` tears it all down.
pub(crate) struct SimHandle {
    cancel: CancellationToken,
    engine_thread: Option<JoinHandle<()>>,
    client_thread: Option<JoinHandle<()>>,
    pub status: Arc<Mutex<SimStatus>>,
    pub tcp_port: u16,
    pub udp_port: u16,
}

impl SimHandle {
    /// Explicit stop (consumes self). Equivalent to dropping the handle.
    pub(crate) fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.cancel.cancel();
        if let Some(h) = self.engine_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.client_thread.take() {
            let _ = h.join();
        }
        // Best-effort: clear the status file so the next poll shows
        // engine-stopped rather than a stale "Connected".
        delete_stale_status();
    }

    /// Snapshot the worker error, if any.
    pub(crate) fn error(&self) -> Option<String> {
        self.status.lock().ok().and_then(|s| s.error.clone())
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn set_err(status: &Arc<Mutex<SimStatus>>, msg: String) {
    log::error!("simulation: {msg}");
    if let Ok(mut s) = status.lock() {
        if s.error.is_none() {
            s.error = Some(msg);
        }
    }
}

/// Build the in-process engine config. Loads `config/default.toml` when present
/// (dev / repo-root launch), else falls back to `AppConfig::default()`. Ports
/// are overridden with OS-assigned free ports so a real engine on 9944/9945 is
/// never disturbed, and synthetic audio is enabled so the audio subsystem
/// indicator lights up in the UI.
fn load_sim_config(tcp_port: u16, udp_port: u16) -> AppConfig {
    let mut config = AppConfig::load("config/default.toml").unwrap_or_else(|e| {
        log::info!("simulation: config/default.toml not loaded ({e}); using defaults");
        AppConfig::default()
    });
    for err in config.validate() {
        log::warn!("simulation config: {err}");
    }
    config.network.tcp_port = tcp_port;
    config.network.udp_port = udp_port;
    // Synthetic audio so the full pipeline (incl. Opus over UDP) is exercised
    // and the companion's audio indicator reflects a live audio path.
    config.audio.enabled = true;
    config.audio.synthetic_source = "sine".to_string();
    config
}

/// Start a simulation. Returns an error string suitable for the UI banner if
/// the engine or threads cannot be brought up.
pub(crate) fn start() -> Result<SimHandle, String> {
    // Fresh ports each call → re-entrant starts cannot collide.
    let (tcp_port, udp_port) = pick_free_ports();
    // Remove any leftover status.json (prior real or sim session) so PIN
    // discovery only sees this engine's fresh write.
    delete_stale_status();

    let config = load_sim_config(tcp_port, udp_port);
    let codec = config.video.codec;
    let framerate = config.video.framerate.max(1);
    let frame_period = Duration::from_secs_f64(1.0 / framerate as f64);
    // 1 IDR/sec, matching headless.rs and the engine's adaptive cadence.
    let gop_size = framerate;

    let cancel = CancellationToken::new();
    let status = Arc::new(Mutex::new(SimStatus::default()));

    // ---- Engine thread: owns StreamingEngine, pumps synthetic NALs ----
    let engine_cancel = cancel.clone();
    let engine_status = status.clone();
    let engine_thread = std::thread::Builder::new()
        .name("fvp-sim-engine".into())
        .spawn(move || {
            let engine = match StreamingEngine::new(config) {
                Ok(e) => e,
                Err(e) => {
                    set_err(&engine_status, format!("engine start failed: {e}"));
                    return;
                }
            };
            // Engine writes status.json("waiting", pin) itself; the companion's
            // existing poll loop reads it.
            let mut stream = SyntheticNalStream::new(codec, gop_size);
            let mut next_tick = Instant::now();
            while !engine_cancel.is_cancelled() {
                let synth = stream.next_frame();
                let _ = engine.submit_frame(EncodedFrame {
                    frame_index: synth.frame_index,
                    nal_data: synth.bytes,
                    is_idr: synth.is_idr,
                    timestamps: FrameTimestamps::new(synth.frame_index),
                });
                // Drift-free pacing, but cap the sleep so cancellation is
                // observed within ~one frame.
                next_tick += frame_period;
                let now = Instant::now();
                if next_tick > now {
                    std::thread::sleep((next_tick - now).min(frame_period));
                } else {
                    next_tick = now;
                }
            }
            engine.shutdown();
        })
        .map_err(|e| format!("spawn engine thread: {e}"))?;

    // ---- Client thread: owns a tokio runtime, runs the mock HMD client ----
    let client_cancel = cancel.clone();
    let client_status = status.clone();
    let client_thread = std::thread::Builder::new()
        .name("fvp-sim-client".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    set_err(&client_status, format!("tokio runtime: {e}"));
                    return;
                }
            };
            // Reconnect loop: the engine re-publishes a new PIN per accept, so
            // if a session ends we re-resolve and reconnect instead of ending
            // the simulation.
            while !client_cancel.is_cancelled() {
                let pin = match wait_for_pin(Duration::from_secs(5)) {
                    Some(p) => p,
                    None => {
                        if client_cancel.is_cancelled() {
                            break;
                        }
                        set_err(&client_status, "engine never published a PIN".into());
                        return;
                    }
                };
                let mut cfg = MockClientConfig::from_ports(
                    std::net::Ipv4Addr::LOCALHOST.into(),
                    tcp_port,
                    udp_port,
                    pin,
                );
                cfg.duration = None; // run until cancelled
                cfg.receive_audio = true;
                match rt.block_on(run_mock_client(cfg, client_cancel.clone())) {
                    Ok(_) => {
                        if client_cancel.is_cancelled() {
                            break;
                        }
                        // Session ended cleanly (engine recycled) — brief
                        // backoff, then reconnect with a fresh PIN.
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    Err(e) => {
                        if client_cancel.is_cancelled() {
                            break;
                        }
                        set_err(&client_status, format!("mock client: {e}"));
                        return;
                    }
                }
            }
        })
        .map_err(|e| format!("spawn client thread: {e}"))?;

    log::info!("simulation started (tcp={tcp_port} udp={udp_port})");
    Ok(SimHandle {
        cancel,
        engine_thread: Some(engine_thread),
        client_thread: Some(client_thread),
        status,
        tcp_port,
        udp_port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_sim_config_sets_ports_and_audio() {
        let mut cfg = load_sim_config(40100, 40101);
        assert_eq!(cfg.network.tcp_port, 40100);
        assert_eq!(cfg.network.udp_port, 40101);
        assert!(cfg.audio.enabled);
        assert_eq!(cfg.audio.synthetic_source, "sine");
        // Config derived from defaults must be valid (no clamped fields).
        assert!(cfg.validate().is_empty(), "sim config should validate cleanly");
    }

    #[test]
    fn pick_free_ports_distinct() {
        let (tcp, udp) = pick_free_ports();
        assert_ne!(tcp, udp);
        assert!(tcp >= 1024 && udp >= 1024);
    }

    /// Full in-process round trip: start the simulation, wait until the engine
    /// + mock client reach Connected (via the real status.json), assert no
    /// worker error, then stop and join. The GUI-free analogue of
    /// streaming-engine's headless_e2e_test. Writes the process-global
    /// status.json, so run the suite with `--test-threads=1`.
    #[test]
    fn sim_smoke_round_trip() {
        use crate::status_parser::{parse_status_json, ConnectionStatus};
        use streaming_engine::simulator::test_helpers::status_path;

        let handle = start().expect("simulation should start");

        let deadline = Instant::now() + Duration::from_secs(15);
        let mut connected = false;
        while Instant::now() < deadline {
            if let Some(err) = handle.error() {
                panic!("simulation worker error: {err}");
            }
            if let Some(path) = status_path() {
                if let Ok(c) = std::fs::read_to_string(&path) {
                    if let Some(p) = parse_status_json(&c) {
                        if p.connection == ConnectionStatus::Connected {
                            connected = true;
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(connected, "engine never reached Connected within 15s");
        assert!(handle.error().is_none(), "unexpected worker error after connect");
        handle.stop(); // cancels + joins both threads
    }
}
