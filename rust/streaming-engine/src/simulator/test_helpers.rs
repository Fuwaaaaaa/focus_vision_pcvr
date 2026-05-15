//! Shared helpers (pick_free_ports, wait_for_pin, delete_stale_status)
//! promoted from `tests/headless_e2e_test.rs` so both the existing E2E
//! test and the new scenario runner can share them.
//!
//! All tests using these helpers must run with `--test-threads=1` because
//! they share `%APPDATA%/FocusVisionPCVR/status.json` for PIN discovery.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Bind temporary TCP and UDP sockets at port 0 to discover free ports,
/// then drop them so the caller can rebind. Brief TOCTOU window but
/// fine on quiet test runners.
pub fn pick_free_ports() -> (u16, u16) {
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_port = udp.local_addr().unwrap().port();
    let udp_port = if udp_port == tcp_port { udp_port.wrapping_add(1).max(1024) } else { udp_port };
    drop(tcp);
    drop(udp);
    (tcp_port, udp_port)
}

/// Bind a UDP socket at port 0, capture the OS-assigned port, then drop
/// the socket. Used by scenarios that need a free UDP port for an OSC
/// loopback receiver (the same port must be configured as
/// `face_tracking.osc_port` so the engine targets it).
pub fn pick_free_udp_port() -> u16 {
    let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = s.local_addr().unwrap().port();
    drop(s);
    port
}

/// Like `pick_free_udp_port` but rejects any port whose value appears in
/// `exclude`. Necessary because Windows often allocates sequential
/// ephemeral ports — without this, the OSC loopback would land on the
/// same port as the mock client's video/audio/tracking UDP bind and
/// silently swallow RTP packets that should have reached the video
/// receiver.
pub fn pick_free_udp_port_excluding(exclude: &[u16]) -> u16 {
    for _ in 0..256 {
        let p = pick_free_udp_port();
        if !exclude.contains(&p) {
            return p;
        }
    }
    // Extremely unlikely — would mean 256 sequential picks all collided.
    // Fall back to a non-colliding offset so the caller doesn't hang.
    let mut p = pick_free_udp_port();
    while exclude.contains(&p) {
        p = p.wrapping_add(1).max(1024);
    }
    p
}

/// Path to status.json. The engine writes here on each
/// `TcpControlServer::new()` call (see `engine::run_streaming`).
pub fn status_path() -> Option<PathBuf> {
    dirs_next::data_dir().map(|d| d.join("FocusVisionPCVR").join("status.json"))
}

/// Delete any stale status.json before launching an engine. Without this
/// a prior test run leaves a file behind and `wait_for_pin` happily reads
/// the OLD pin while the engine is still starting up.
pub fn delete_stale_status() {
    if let Some(p) = status_path() {
        let _ = std::fs::remove_file(&p);
    }
}

/// Poll status.json until it has a non-placeholder PIN, or time out.
/// Caller must have called `delete_stale_status()` first.
pub fn wait_for_pin(timeout: Duration) -> Option<u32> {
    let path = status_path()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(s) = v["pin"].as_str() {
                    if s != "------" {
                        if let Ok(p) = s.parse::<u32>() {
                            return Some(p);
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_free_ports_returns_distinct() {
        let (tcp, udp) = pick_free_ports();
        assert_ne!(tcp, udp);
        assert!(tcp > 1024 && udp > 1024);
    }

    #[test]
    fn status_path_under_data_dir() {
        let p = status_path().unwrap();
        assert!(p.to_string_lossy().contains("FocusVisionPCVR"));
        assert!(p.file_name().unwrap() == "status.json");
    }

    #[test]
    fn wait_for_pin_times_out_when_no_file() {
        delete_stale_status();
        let start = Instant::now();
        let result = wait_for_pin(Duration::from_millis(200));
        assert!(result.is_none());
        assert!(start.elapsed() >= Duration::from_millis(200));
    }
}
