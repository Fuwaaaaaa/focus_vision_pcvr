//! focus-vision-mock-client — synthetic HMD-side client for the simulator.
//!
//! Pairs with `focus-vision-headless` (B3) to exercise the engine's full
//! transport pipeline end-to-end without a real Focus Vision headset.
//! Connects TCP+TLS, completes the HELLO → PIN → STREAM_CONFIG handshake,
//! receives RTP video on UDP, sends HEARTBEAT_ACK back, prints summary
//! stats on exit.
//!
//! Gated behind the `simulator` feature.
//!
//! Usage:
//!     focus-vision-mock-client --pin 123456                # default 127.0.0.1:9944
//!     focus-vision-mock-client --server 10.0.0.1:9944 --pin 123456
//!     focus-vision-mock-client --pin 123456 --duration 5   # exit after 5 s
//!     focus-vision-mock-client --pin-file path/to/status.json

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use streaming_engine::simulator::{run, MockClientConfig};
use tokio_util::sync::CancellationToken;

struct Args {
    server: SocketAddr,
    udp_port: u16,
    pin: Option<u32>,
    pin_file: Option<PathBuf>,
    duration: Option<Duration>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut server = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            fvp_common::DEFAULT_TCP_PORT,
        );
        let mut udp_port = fvp_common::DEFAULT_UDP_PORT;
        let mut pin: Option<u32> = None;
        let mut pin_file: Option<PathBuf> = None;
        let mut duration: Option<Duration> = None;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--server" => {
                    let s = args.next()
                        .ok_or_else(|| "--server needs HOST:PORT".to_string())?;
                    server = s.parse()
                        .map_err(|e| format!("--server parse: {}", e))?;
                }
                "--udp-port" => {
                    let s = args.next()
                        .ok_or_else(|| "--udp-port needs a port".to_string())?;
                    udp_port = s.parse()
                        .map_err(|e| format!("--udp-port: {}", e))?;
                }
                "--pin" => {
                    let s = args.next()
                        .ok_or_else(|| "--pin needs 6 digits".to_string())?;
                    pin = Some(s.parse::<u32>()
                        .map_err(|e| format!("--pin: {}", e))?);
                }
                "--pin-file" => {
                    pin_file = Some(args.next()
                        .ok_or_else(|| "--pin-file needs a path".to_string())?
                        .into());
                }
                "--duration" => {
                    let s = args.next()
                        .ok_or_else(|| "--duration needs seconds".to_string())?;
                    duration = Some(Duration::from_secs(s.parse::<u64>()
                        .map_err(|e| format!("--duration: {}", e))?));
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {}", other)),
            }
        }
        Ok(Self { server, udp_port, pin, pin_file, duration })
    }
}

fn print_help() {
    let bin = std::env::args().next().unwrap_or_else(|| "focus-vision-mock-client".to_string());
    println!(
        "{} v{} — simulator-only HMD-side mock client\n",
        bin, env!("CARGO_PKG_VERSION")
    );
    println!("Usage: {} [--server HOST:PORT] [--udp-port N] (--pin DIGITS | --pin-file PATH)", bin);
    println!("       [--duration SEC]\n");
    println!("  --server HOST:PORT  Engine TCP endpoint (default 127.0.0.1:9944)");
    println!("  --udp-port N        Base UDP port (video=N+1, tracking=N+2, audio=N+3) (default 9945)");
    println!("  --pin DIGITS        6-digit PIN displayed by the engine");
    println!("  --pin-file PATH     Read PIN from status.json (engine writes it on startup)");
    println!("  --duration SEC      Disconnect after N seconds (default: run forever)");
    println!("  -h, --help          Show this help");
}

/// Pull the PIN out of the engine's status.json. The engine writes it as
/// a 6-character string under the `pin` key during `waiting` state.
fn read_pin_from_status(path: &PathBuf) -> Result<u32, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("read {:?}: {}", path, e))?;
    let v: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("parse {:?}: {}", path, e))?;
    let pin_str = v["pin"]
        .as_str()
        .ok_or_else(|| format!("{:?}: no 'pin' string", path))?;
    if pin_str == "------" {
        return Err(format!("{:?}: engine has not yet displayed a PIN", path));
    }
    pin_str.parse::<u32>()
        .map_err(|e| format!("{:?}: PIN '{}' not an integer: {}", path, pin_str, e))
}

/// Resolve PIN from the explicit --pin flag, or fall back to --pin-file,
/// or finally the engine's default status.json path under %APPDATA%.
fn resolve_pin(args: &Args) -> Result<u32, String> {
    if let Some(p) = args.pin {
        return Ok(p);
    }
    if let Some(ref pf) = args.pin_file {
        return read_pin_from_status(pf);
    }
    // Default: %APPDATA%/FocusVisionPCVR/status.json on Windows.
    if let Some(d) = dirs_next::data_dir() {
        let path = d.join("FocusVisionPCVR").join("status.json");
        return read_pin_from_status(&path);
    }
    Err("no PIN source: pass --pin or --pin-file".into())
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

    let pin = match resolve_pin(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error resolving PIN: {}", e);
            return ExitCode::from(2);
        }
    };

    log::info!("focus-vision-mock-client v{} starting", env!("CARGO_PKG_VERSION"));

    let mut config = MockClientConfig::from_ports(args.server.ip(), args.server.port(), args.udp_port, pin);
    config.duration = args.duration;

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("tokio runtime build failed: {}", e);
            return ExitCode::from(1);
        }
    };

    // No Ctrl+C handler — the simulator's tokio build does not enable the
    // `signal` feature. Mock-client is normally bounded by --duration in
    // tests and CI; interactive use can SIGKILL the process instead.
    let cancel = CancellationToken::new();

    let stats = match runtime.block_on(run(config, cancel)) {
        Ok(s) => s,
        Err(e) => {
            log::error!("mock-client run failed: {}", e);
            return ExitCode::from(1);
        }
    };

    println!("--- mock-client stats ---");
    println!("connect duration:    {:?}", stats.connect_duration);
    println!("stream duration:     {:?}", stats.stream_duration);
    println!("video packets:       {}", stats.video_packets_received);
    println!("frames decoded:      {}", stats.frames_decoded);
    println!("IDR frames seen:     {}", stats.idr_frames_seen);
    println!("heartbeats sent:     {}", stats.heartbeats_sent);

    ExitCode::SUCCESS
}
