//! Shared helpers (pick_free_ports, wait_for_pin, delete_stale_status)
//! promoted from `tests/headless_e2e_test.rs` so both the existing E2E
//! test and the new scenario runner can share them.
//!
//! All tests using these helpers must run with `--test-threads=1` because
//! they share `%APPDATA%/FocusVisionPCVR/status.json` for PIN discovery.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Reserve a contiguous block of free ports for an in-process engine + mock
/// client pair, returning `(tcp_port, udp_port)`.
///
/// The block is chosen from a LOW, non-ephemeral range on purpose. The mock
/// client binds the *fixed* `udp_port + 1/2/3` (video/tracking/audio) ports,
/// while the engine opens additional *ephemeral* (`0.0.0.0:0`) sender sockets
/// (`UdpSender::new`). If the block lived inside the OS dynamic/ephemeral range,
/// those sender sockets could be assigned the very ports the client must bind —
/// Windows readily recycles a just-freed ephemeral port — producing a
/// deterministic `WSAEADDRINUSE (os error 10048)` on the client's receiver
/// bind. Keeping the block below the dynamic range (Windows ≥ 49152,
/// Linux ≥ 32768) ensures the ephemeral allocator never hands out our service
/// ports, so the engine's senders and the client's receivers can't collide.
pub fn pick_free_ports() -> (u16, u16) {
    // 20000..30000 sits below both the Windows and Linux default dynamic
    // ranges. step_by(8) leaves a gap between adjacent 5-port blocks so a
    // partially-occupied neighbour can't alias into the next probe.
    for base in (20000u16..30000).step_by(8) {
        if try_reserve_block(base) {
            // tcp_port = base; udp base = base + 1 (video/tracking/audio are
            // udp+1/+2/+3). The whole block was bindable just above; the brief
            // gap before the engine/client rebind is safe because nothing else
            // in the test binds low ports.
            return (base, base + 1);
        }
    }
    // Fallback: OS-assigned ephemeral probe (original behaviour). Reached only
    // if every low block is occupied, which on a test runner is effectively
    // impossible.
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp_port = tcp.local_addr().unwrap().port();
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_port = udp.local_addr().unwrap().port();
    let udp_port = if udp_port == tcp_port { udp_port.wrapping_add(1).max(1024) } else { udp_port };
    drop(tcp);
    drop(udp);
    (tcp_port, udp_port)
}

/// Try to simultaneously bind every port a session based at `base` uses:
/// TCP control on `base`, and UDP on `base+1..=base+4` (udp base plus the
/// video/tracking/audio offsets). Binds on `0.0.0.0` to match the engine's and
/// mock client's real bind addresses, so success guarantees the port is free on
/// every interface (incl. 127.0.0.1). All sockets drop before returning so the
/// caller can rebind. Returns true iff the whole block was free.
fn try_reserve_block(base: u16) -> bool {
    if base.checked_add(4).is_none() {
        return false;
    }
    let tcp = match std::net::TcpListener::bind(("0.0.0.0", base)) {
        Ok(l) => l,
        Err(_) => return false,
    };
    let mut udps = Vec::with_capacity(4);
    for off in 1..=4u16 {
        match std::net::UdpSocket::bind(("0.0.0.0", base + off)) {
            Ok(s) => udps.push(s),
            Err(_) => return false, // tcp + any earlier udp sockets drop here
        }
    }
    drop(udps);
    drop(tcp);
    true
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
