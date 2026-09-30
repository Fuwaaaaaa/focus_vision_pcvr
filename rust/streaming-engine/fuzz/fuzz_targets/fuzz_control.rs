#![no_main]

//! What the headset sends after pairing, and the tracking datagrams: every
//! parser the engine runs on them must take any bytes without panicking.
//! (The stable-toolchain counterpart is tests/fuzz_tests.rs.)

use fvp_common::protocol;
use libfuzzer_sys::fuzz_target;
use streaming_engine::engine::{parse_config_update, parse_heartbeat, take_control_message};
use streaming_engine::face_tracking::osc_bridge::parse_face_data;
use streaming_engine::tracking::receiver::{parse_controller, parse_head_pose};

fuzz_target!(|data: &[u8]| {
    // The TCP framing: messages come off the front until the stream stops
    // or is refused (a length past MAX_MSG_LEN).
    let mut inbox = data.to_vec();
    while let Ok(Some(message)) = take_control_message(&mut inbox) {
        assert!(!message.is_empty() && message.len() <= fvp_common::MAX_MSG_LEN);
        let payload = &message[1..];
        let _ = parse_heartbeat(payload);
        let _ = parse_config_update(payload);
        let _ = parse_face_data(payload);
        let _ = protocol::parse_view_config(payload);
        let _ = protocol::parse_transport_feedback(payload);
    }

    // Each parser on the raw bytes too.
    let _ = parse_heartbeat(data);
    let _ = parse_config_update(data);
    let _ = parse_face_data(data);
    if let Some(view) = protocol::parse_view_config(data) {
        assert!(view.ipd_m.is_finite() && view.ipd_m > 0.0);
    }
    if let Some((_, rest)) = protocol::frame_pose::parse(data) {
        assert_eq!(rest.len() + protocol::frame_pose::LEN, data.len());
    }
    let _ = parse_head_pose(data);
    let _ = parse_controller(data);
});
