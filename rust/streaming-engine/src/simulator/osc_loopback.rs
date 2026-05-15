//! VRChat OSC loopback receiver for face-tracking E2E assertions.
//!
//! Binds a UDP socket on a caller-supplied port (the same one set as
//! `face_tracking.osc_port` in `AppConfig`) and decodes incoming OSC
//! float messages so scenario assertions can check that the engine's
//! `OscBridge` emitted the expected `/avatar/parameters/*` traffic.
//!
//! Only handles the minimal "address + ,f + float" subset of OSC that
//! the production bridge sends — bundles, type tag arrays, blobs, etc.
//! are out of scope.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

/// Captured OSC traffic, keyed by address string. Each entry is the
/// sequence of float values received for that address.
pub type OscCapture = HashMap<String, Vec<f32>>;

/// Parse a single OSC float message:
/// `address\0[pad to 4]` + `,f\0\0` + `float (big-endian)`.
/// Returns `None` if the buffer doesn't match the minimal shape.
pub fn parse_osc_float(buf: &[u8]) -> Option<(String, f32)> {
    let addr_end = buf.iter().position(|&b| b == 0)?;
    let addr = std::str::from_utf8(&buf[..addr_end]).ok()?.to_string();
    // OSC pads the address string (including its null) to a 4-byte boundary.
    let after_addr = (addr_end + 4) & !3;
    if buf.len() < after_addr + 8 {
        return None;
    }
    if &buf[after_addr..after_addr + 4] != b",f\0\0" {
        return None;
    }
    let val_bytes: [u8; 4] = buf[after_addr + 4..after_addr + 8].try_into().ok()?;
    Some((addr, f32::from_be_bytes(val_bytes)))
}

/// UDP loopback that captures OSC float messages sent by the engine's
/// `OscBridge`. Bind to the same port the engine targets, drive `run()`
/// until cancelled, then read `snapshot()` for assertion.
pub struct OscLoopback {
    socket: UdpSocket,
    captured: Arc<Mutex<OscCapture>>,
}

impl OscLoopback {
    /// Bind a loopback receiver on the given port. Port 0 lets the OS
    /// choose — useful when the caller plans to read `local_port()` and
    /// pass it into `face_tracking.osc_port`.
    pub async fn bind(port: u16) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("127.0.0.1", port)).await?;
        Ok(Self {
            socket,
            captured: Arc::new(Mutex::new(OscCapture::new())),
        })
    }

    pub fn local_port(&self) -> u16 {
        self.socket.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    /// Handle to the shared capture map. Snapshots can be taken
    /// concurrently while `run()` is still active.
    pub fn capture_handle(&self) -> Arc<Mutex<OscCapture>> {
        self.captured.clone()
    }

    /// Drain incoming packets until `cancel` fires. Each parseable OSC
    /// float message appends to the address's value vector.
    ///
    /// Recoverable recv errors (WSAEMSGSIZE on Windows when prior UDP traffic
    /// raced the bind, transient WouldBlock, etc.) are logged and skipped
    /// rather than breaking the loop — the loopback is a best-effort capture,
    /// and breaking on the first transient error stranded all subsequent
    /// in-flight packets in earlier iterations.
    pub async fn run(&self, cancel: CancellationToken) {
        let mut buf = [0u8; 2048];
        loop {
            tokio::select! {
                r = self.socket.recv_from(&mut buf) => match r {
                    Ok((n, peer)) => {
                        match parse_osc_float(&buf[..n]) {
                            Some((addr, value)) => {
                                log::trace!("OSC loopback recv {}B from {} -> {} = {}", n, peer, addr, value);
                                if let Ok(mut guard) = self.captured.lock() {
                                    guard.entry(addr).or_default().push(value);
                                }
                            }
                            None => {
                                log::warn!("OSC loopback recv {}B from {} — parse failed (first bytes: {:?})", n, peer, &buf[..n.min(16)]);
                            }
                        }
                    }
                    Err(e) => {
                        log::debug!("OSC loopback recv skipped: {}", e);
                        // Brief sleep so we don't hot-loop on a persistent
                        // error and starve sibling tasks (video receiver,
                        // tracking sender) on the single-thread runtime.
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                },
                _ = cancel.cancelled() => break,
            }
        }
    }

    /// Snapshot the captured map (clones internally — caller can examine
    /// without holding the mutex).
    pub fn snapshot(&self) -> OscCapture {
        self.captured.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Encode a float OSC message the same way `osc_bridge::encode_osc_float`
    /// does, so we can drive the loopback without touching production code.
    fn encode_osc(address: &str, value: f32) -> Vec<u8> {
        let mut msg = Vec::with_capacity(64);
        msg.extend_from_slice(address.as_bytes());
        msg.push(0);
        while msg.len() % 4 != 0 {
            msg.push(0);
        }
        msg.extend_from_slice(b",f\0\0");
        msg.extend_from_slice(&value.to_be_bytes());
        msg
    }

    #[test]
    fn parse_simple_jaw_open() {
        let msg = encode_osc("/avatar/parameters/JawOpen", 0.75);
        let (addr, val) = parse_osc_float(&msg).expect("parse");
        assert_eq!(addr, "/avatar/parameters/JawOpen");
        assert!((val - 0.75).abs() < 1e-6);
    }

    #[test]
    fn parse_short_address_with_padding() {
        let msg = encode_osc("/x", 0.5);
        let (addr, val) = parse_osc_float(&msg).expect("parse");
        assert_eq!(addr, "/x");
        assert!((val - 0.5).abs() < 1e-6);
    }

    #[test]
    fn reject_wrong_type_tag() {
        let mut msg = encode_osc("/foo", 1.0);
        // Corrupt the type tag from ",f\0\0" to ",i\0\0".
        let null_pos = msg.iter().position(|&b| b == 0).unwrap();
        let tag_start = (null_pos + 4) & !3;
        msg[tag_start + 1] = b'i';
        assert!(parse_osc_float(&msg).is_none());
    }

    #[test]
    fn reject_truncated_buffer() {
        let msg = encode_osc("/foo", 1.0);
        assert!(parse_osc_float(&msg[..msg.len() - 2]).is_none());
    }

    #[tokio::test]
    async fn loopback_captures_round_tripped_message() {
        let loopback = OscLoopback::bind(0).await.unwrap();
        let port = loopback.local_port();
        assert_ne!(port, 0);

        let cancel = CancellationToken::new();
        let cancel_for_run = cancel.clone();
        let handle_capture = loopback.capture_handle();

        let runner = tokio::spawn(async move {
            loopback.run(cancel_for_run).await;
        });

        // Send two messages to the loopback port.
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = format!("127.0.0.1:{}", port);
        sender.send_to(&encode_osc("/a/b", 0.25), &target).await.unwrap();
        sender.send_to(&encode_osc("/a/b", 0.75), &target).await.unwrap();

        // Give the receiver a moment to drain both packets.
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
        let _ = runner.await;

        let snap = handle_capture.lock().unwrap().clone();
        let values = snap.get("/a/b").expect("address captured");
        assert_eq!(values.len(), 2);
        assert!((values[0] - 0.25).abs() < 1e-6);
        assert!((values[1] - 0.75).abs() < 1e-6);
    }
}
