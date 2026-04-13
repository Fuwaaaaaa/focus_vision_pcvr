#![no_main]

use libfuzzer_sys::fuzz_target;
use fvp_common::protocol;

fuzz_target!(|data: &[u8]| {
    // Fuzz parse_hello_version — must not panic on any input
    let _ = protocol::parse_hello_version(data);

    // Fuzz parse_transport_feedback — must not panic on any input
    let _ = protocol::parse_transport_feedback(data);

    // Fuzz encode/decode roundtrip for transport feedback
    // Generate entries from the fuzzed data and verify roundtrip
    if data.len() >= 6 {
        let count = u16::from_le_bytes([data[0], data[1]]) as usize;
        if count > 0 && count <= 256 && data.len() >= 2 + count * 6 {
            // Parse, then re-encode and verify
            if let Some(entries) = protocol::parse_transport_feedback(data) {
                let re_encoded = protocol::encode_transport_feedback(&entries);
                let re_parsed = protocol::parse_transport_feedback(&re_encoded);
                assert!(re_parsed.is_some(), "Roundtrip failed: encode then parse returned None");
                let re_entries = re_parsed.unwrap();
                assert_eq!(entries.len(), re_entries.len(), "Roundtrip entry count mismatch");
            }
        }
    }

    // Fuzz fvp_flags encode/decode roundtrip
    if data.len() >= 2 {
        let flags = u16::from_le_bytes([data[0], data[1]]);
        let kf = protocol::fvp_flags::is_keyframe(flags);
        let si = protocol::fvp_flags::slice_index(flags);
        let sc = protocol::fvp_flags::slice_count(flags);
        let sid = protocol::fvp_flags::stream_id(flags);

        // Re-encode and verify fields survive roundtrip
        let re_flags = protocol::fvp_flags::encode(kf, si, sc, sid);
        assert_eq!(protocol::fvp_flags::is_keyframe(re_flags), kf);
        assert_eq!(protocol::fvp_flags::slice_index(re_flags), si);
        assert_eq!(protocol::fvp_flags::slice_count(re_flags), sc);
        assert_eq!(protocol::fvp_flags::stream_id(re_flags), sid);
    }
});
