//! End-to-end UDP loopback test for the OSC bridge.
//!
//! Exercises the full path that production runs: blendshape input →
//! EMA smoothing → profile weighting → OSC encoding → UDP send. A receiver
//! is bound on `127.0.0.1:0` (kernel-assigned port) and `OscBridge::set_target`
//! routes the outgoing packets there, so we observe the exact bytes VRChat
//! would receive in production.
//!
//! The OSC parser is intentionally inlined into this test file: keeping it
//! out of the production crate means a future refactor of the encoder can
//! be caught by a divergence here, instead of by a parser that mirrors the
//! encoder's bugs.

use std::collections::HashMap;
use std::net::UdpSocket;
use std::time::Duration;

use streaming_engine::face_tracking::osc_bridge::OscBridge;

/// Bind a loopback receiver and return (socket, "127.0.0.1:N").
fn bind_receiver() -> (UdpSocket, String) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind receiver");
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("set timeout");
    let addr = socket.local_addr().expect("local_addr");
    (socket, addr.to_string())
}

/// Drain everything the bridge sent in this batch. Stops once `recv_from`
/// returns WouldBlock (timeout).
fn drain(socket: &UdpSocket) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 256];
    while let Ok((n, _)) = socket.recv_from(&mut buf) {
        out.push(buf[..n].to_vec());
    }
    out
}

/// Decode one OSC message: address string (null-terminated, 4-byte padded)
/// plus a `",f\0\0"` type tag plus a 4-byte big-endian float. Returns
/// `Some((address, value))` or `None` if the bytes don't match the format.
fn parse_osc_float(bytes: &[u8]) -> Option<(String, f32)> {
    // Find the null terminator of the address.
    let null = bytes.iter().position(|&b| b == 0)?;
    let addr = std::str::from_utf8(&bytes[..null]).ok()?.to_string();
    // Address is padded to a 4-byte boundary.
    let pad_to = ((null + 1) + 3) & !3;
    if bytes.len() < pad_to + 8 {
        return None;
    }
    // Type tag ",f\0\0"
    if &bytes[pad_to..pad_to + 4] != b",f\0\0" {
        return None;
    }
    let v = f32::from_be_bytes([
        bytes[pad_to + 4],
        bytes[pad_to + 5],
        bytes[pad_to + 6],
        bytes[pad_to + 7],
    ]);
    Some((addr, v))
}

#[test]
fn lip_blendshapes_traverse_udp_to_vrchat_format() {
    let (rx, target) = bind_receiver();
    // Smoothing = 0 means raw values pass through verbatim — that lets us
    // assert on exact numerics rather than chasing EMA-state convergence.
    let mut bridge = OscBridge::with_smoothing(0.0);
    bridge.set_target(target);

    let mut lip = [0.0f32; 37];
    let eye = [0.0f32; 14];
    // JawOpen (index 3) ~ half-open mouth.
    lip[3] = 0.5;
    // MouthSmileRight (index 12) ~ small smile.
    lip[12] = 0.25;

    bridge.send_face_data(true, false, &lip, &eye);

    let packets = drain(&rx);
    let parsed: HashMap<String, f32> = packets
        .iter()
        .filter_map(|p| parse_osc_float(p))
        .collect();

    // Each non-zero blendshape > 0.01 yields one OSC packet.
    assert_eq!(parsed.len(), 2, "got {} packets, want 2", parsed.len());

    let jaw = parsed
        .get("/avatar/parameters/JawOpen")
        .expect("JawOpen missing");
    assert!((jaw - 0.5).abs() < 1e-3);

    let smile = parsed
        .get("/avatar/parameters/MouthSmileRight")
        .expect("MouthSmileRight missing");
    assert!((smile - 0.25).abs() < 1e-3);
}

#[test]
fn eye_blendshapes_use_separate_name_table() {
    let (rx, target) = bind_receiver();
    let mut bridge = OscBridge::with_smoothing(0.0);
    bridge.set_target(target);

    let lip = [0.0f32; 37];
    let mut eye = [0.0f32; 14];
    // EyeLeftBlink (eye index 0)
    eye[0] = 0.8;
    // EyeRightBlink (eye index 6)
    eye[6] = 0.7;

    bridge.send_face_data(false, true, &lip, &eye);

    let packets = drain(&rx);
    let parsed: HashMap<String, f32> = packets
        .iter()
        .filter_map(|p| parse_osc_float(p))
        .collect();

    assert!(parsed.contains_key("/avatar/parameters/EyeLeftBlink"));
    assert!(parsed.contains_key("/avatar/parameters/EyeRightBlink"));
    // Sanity-check value preservation on at least one.
    let blink = parsed
        .get("/avatar/parameters/EyeLeftBlink")
        .expect("EyeLeftBlink missing");
    assert!((blink - 0.8).abs() < 1e-3);
}

#[test]
fn ema_smoothing_attenuates_first_frame() {
    // With α=0.6, the first frame's smoothed = 0.6 * 0.0 + 0.4 * raw = 0.4 * raw.
    // A 0.5 input → 0.2 smoothed → above the 0.01 send threshold but
    // visibly attenuated from raw. Confirms the EMA path runs on every
    // send, not just after a "warm-up" frame.
    let (rx, target) = bind_receiver();
    let mut bridge = OscBridge::with_smoothing(0.6);
    bridge.set_target(target);

    let mut lip = [0.0f32; 37];
    let eye = [0.0f32; 14];
    lip[3] = 0.5; // JawOpen

    bridge.send_face_data(true, false, &lip, &eye);

    let packets = drain(&rx);
    let parsed: HashMap<String, f32> = packets
        .iter()
        .filter_map(|p| parse_osc_float(p))
        .collect();

    let jaw = parsed
        .get("/avatar/parameters/JawOpen")
        .expect("JawOpen missing");
    assert!((jaw - 0.2).abs() < 1e-3, "expected ~0.2 (EMA-attenuated), got {}", jaw);
}

#[test]
fn values_below_threshold_are_dropped() {
    let (rx, target) = bind_receiver();
    let mut bridge = OscBridge::with_smoothing(0.0);
    bridge.set_target(target);

    let mut lip = [0.0f32; 37];
    let eye = [0.0f32; 14];
    // 0.005 < 0.01 threshold → must not be sent.
    lip[3] = 0.005;

    bridge.send_face_data(true, false, &lip, &eye);

    let packets = drain(&rx);
    assert!(
        packets.is_empty(),
        "expected no packets for below-threshold input, got {}",
        packets.len()
    );
}
