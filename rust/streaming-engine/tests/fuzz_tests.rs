/// Property-based fuzz tests — runs random inputs via `cargo test`.
/// Complements the cargo-fuzz targets in fuzz/ (Linux/CI only).
use rand::RngCore;

const ITERATIONS: usize = 10_000;

fn random_bytes(rng: &mut impl RngCore, max_len: usize) -> Vec<u8> {
    let len = (rng.next_u32() as usize) % max_len;
    let mut buf = vec![0u8; len];
    rng.fill_bytes(&mut buf);
    buf
}

// ---------- RTP ----------

#[test]
fn fuzz_rtp_packetize_no_panic() {
    use streaming_engine::transport::rtp::RtpPacketizer;

    let mut rng = rand::thread_rng();
    let mut packetizer = RtpPacketizer::new(0xDEADBEEF);

    for _ in 0..ITERATIONS {
        let data = random_bytes(&mut rng, 128 * 1024);
        let frame_index = rng.next_u32();
        let timestamp = rng.next_u32();
        let is_keyframe = rng.next_u32().is_multiple_of(2);

        let packets = packetizer.packetize(&data, frame_index, timestamp, is_keyframe);

        for pkt in &packets {
            assert!(pkt.data.len() >= fvp_common::PACKET_HEADER_LEN, "Packet too small: {}", pkt.data.len());
        }

        packetizer.recycle(packets);
    }
}

// ---------- FEC ----------

#[test]
fn fuzz_fec_encode_decode_roundtrip() {
    use streaming_engine::transport::fec::{FecDecoder, FecEncoder};

    let mut rng = rand::thread_rng();

    for _ in 0..1_000 {
        let shard_size = ((rng.next_u32() % 200) as usize).max(1) + 1;
        let shard_count = ((rng.next_u32() % 20) as usize).max(1) + 1;
        let redundancy = (rng.next_u32() % 80 + 5) as f32 / 100.0;

        // Build data shards
        let mut data_shards: Vec<Vec<u8>> = Vec::new();
        for _ in 0..shard_count {
            let mut shard = vec![0u8; shard_size];
            rng.fill_bytes(&mut shard);
            data_shards.push(shard);
        }
        let original = data_shards.clone();

        // Encode
        let mut encoder = FecEncoder::new(redundancy);
        let all_shards = match encoder.encode(data_shards) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let total = all_shards.len();
        let data_count = shard_count;

        // Drop random shards
        let mut shards: Vec<Option<Vec<u8>>> = all_shards.into_iter().map(Some).collect();
        let max_drops = total - data_count;
        let drops = (rng.next_u32() as usize) % (max_drops + 2); // sometimes exceed limit
        for _ in 0..drops {
            let idx = rng.next_u32() as usize % total;
            shards[idx] = None;
        }

        let available = shards.iter().filter(|s| s.is_some()).count();

        // Decode — must not panic
        if let Ok(recovered) = FecDecoder::decode(&mut shards, data_count) {
            if available >= data_count {
                assert_eq!(recovered.len(), data_count);
                for (i, shard) in recovered.iter().enumerate() {
                    assert_eq!(shard, &original[i], "Shard {} mismatch", i);
                }
            }
        }
        // Err is acceptable — decoder may reject under-shard scenarios.
    }
}

#[test]
fn fuzz_fec_reassembler_forged_headers_no_panic() {
    // Receiver-side hardening: packets with random shard_index /
    // shard_count / data_shard_count / flags (including data > total,
    // data = 0, slice_index >= slice_count) and random payload sizes must
    // never panic, and a frame index change must always start cleanly.
    use streaming_engine::pipeline::FecFrameReassembler;
    use streaming_engine::transport::rtp::{write_fvp_header, write_rtp_header};

    let mut rng = rand::thread_rng();
    let mut reassembler = FecFrameReassembler::new();

    for _ in 0..ITERATIONS {
        let mut pkt = Vec::new();
        write_rtp_header(&mut pkt, 97, rng.next_u32() & 1 == 0, 0, 0, 0);
        let small = |v: u32| (v % 24) as u16; // keep counts small so frames can complete
        write_fvp_header(
            &mut pkt,
            rng.next_u32() % 4,
            small(rng.next_u32()),
            small(rng.next_u32()),
            rng.next_u32() as u16,
            small(rng.next_u32()),
        );
        let payload_len = (rng.next_u32() % 64) as usize;
        let mut payload = vec![0u8; payload_len];
        rng.fill_bytes(&mut payload);
        pkt.extend_from_slice(&payload);

        let _ = reassembler.feed(&pkt);
        // Truncated copies exercise the length checks.
        let cut = (rng.next_u32() as usize) % (pkt.len() + 1);
        let _ = reassembler.feed(&pkt[..cut]);
    }
}

// ---------- Protocol ----------

#[test]
fn fuzz_protocol_parsers_no_panic() {
    use fvp_common::protocol;

    let mut rng = rand::thread_rng();

    for _ in 0..ITERATIONS {
        let data = random_bytes(&mut rng, 2048);

        // Must not panic on any input
        let _ = protocol::parse_hello_version(&data);
        let _ = protocol::parse_transport_feedback(&data);

        // fvp_flags roundtrip
        if data.len() >= 2 {
            let flags = u16::from_le_bytes([data[0], data[1]]);
            let kf = protocol::fvp_flags::is_keyframe(flags);
            let si = protocol::fvp_flags::slice_index(flags);
            let sc = protocol::fvp_flags::slice_count(flags);
            let sid = protocol::fvp_flags::stream_id(flags);

            let re = protocol::fvp_flags::encode(kf, si, sc, sid);
            assert_eq!(protocol::fvp_flags::is_keyframe(re), kf);
            assert_eq!(protocol::fvp_flags::slice_index(re), si);
            assert_eq!(protocol::fvp_flags::slice_count(re), sc);
            assert_eq!(protocol::fvp_flags::stream_id(re), sid);
        }

        // transport feedback roundtrip
        if let Some(entries) = protocol::parse_transport_feedback(&data) {
            let encoded = protocol::encode_transport_feedback(&entries);
            let decoded = protocol::parse_transport_feedback(&encoded);
            assert!(decoded.is_some(), "Roundtrip failed");
            assert_eq!(decoded.unwrap().len(), entries.len());
        }
    }
}

// ---------- What the headset sends ----------
//
// REGRESSION: the fuzzing stopped at the HELLO and the FEC primitives; the
// parsers the engine runs on every control message and tracking datagram
// from the headset were not exercised.

#[test]
fn fuzz_headset_message_parsers_no_panic() {
    use fvp_common::protocol;
    use streaming_engine::engine::{parse_config_update, parse_heartbeat};
    use streaming_engine::face_tracking::osc_bridge::parse_face_data;
    use streaming_engine::tracking::receiver::{parse_controller, parse_head_pose};

    let mut rng = rand::thread_rng();

    for _ in 0..ITERATIONS {
        // Around the parsers' sizes, where the bounds checks are.
        let data = random_bytes(&mut rng, 320);

        assert_eq!(parse_heartbeat(&data).is_some(), data.len() >= 26);
        if let Some((key, value)) = parse_config_update(&data) {
            assert_eq!(key, data[0]);
            assert_eq!(value.to_le_bytes(), data[1..5]);
        }
        let _ = parse_face_data(&data);
        if let Some(view) = protocol::parse_view_config(&data) {
            assert!(view.ipd_m.is_finite() && view.ipd_m > 0.0, "IPD {}", view.ipd_m);
            for eye in view.eyes {
                for angle in [eye.left, eye.right, eye.up, eye.down] {
                    assert!(angle.is_finite(), "FOV angle {angle}");
                }
            }
        }
        if let Some((orientation, rest)) = protocol::frame_pose::parse(&data) {
            assert_eq!(rest.len() + protocol::frame_pose::LEN, data.len());
            if let Some(q) = orientation {
                assert!(q.iter().all(|v| v.is_finite()));
            }
        }
        let _ = parse_head_pose(&data);
        let _ = parse_controller(&data);
    }
}

#[test]
fn fuzz_face_data_reaches_vrchat_finite_and_in_range() {
    // FACE_DATA carries raw f32s (NaN, Inf, anything); the OSC bridge must
    // send VRChat only finite values in 0..1, whatever came in.
    use streaming_engine::face_tracking::osc_bridge::{parse_face_data, OscBridge};

    let vrchat = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    vrchat.set_nonblocking(true).unwrap();
    let mut bridge = OscBridge::with_smoothing(0.5);
    bridge.set_target(vrchat.local_addr().unwrap().to_string());

    let mut rng = rand::thread_rng();
    let mut buf = [0u8; 512];
    let mut sent = 0;
    for _ in 0..2_000 {
        let mut payload = vec![0u8; 206];
        rng.fill_bytes(&mut payload);
        // Half the time plausible values, to get past the rest threshold.
        if rng.next_u32().is_multiple_of(2) {
            for at in (2..206).step_by(4) {
                let v = (rng.next_u32() % 1000) as f32 / 999.0;
                payload[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        let (lip_valid, eye_valid, lip, eye) = parse_face_data(&payload).expect("206 bytes");
        bridge.send_face_data(lip_valid, eye_valid, &lip, &eye);

        while let Ok(n) = vrchat.recv(&mut buf) {
            let value = osc_float(&buf[..n]);
            assert!(value.is_finite() && (0.0..=1.0).contains(&value), "sent {value}");
            sent += 1;
        }
    }
    assert!(sent > 0, "nothing reached the receiver");

    /// The float of an OSC message `address\0… ,f\0\0 value` (big-endian).
    fn osc_float(msg: &[u8]) -> f32 {
        let tag = msg.windows(4).position(|w| w == b",f\0\0").expect("a ,f type tag");
        let v = &msg[tag + 4..tag + 8];
        f32::from_be_bytes([v[0], v[1], v[2], v[3]])
    }
}

#[test]
fn fuzz_control_framing_is_the_same_however_it_arrives() {
    // The engine takes messages off a byte stream that TCP splits wherever it
    // likes; a message must come out whole and once, whatever the split.
    use streaming_engine::engine::take_control_message;

    let mut rng = rand::thread_rng();

    for _ in 0..1_000 {
        let messages: Vec<Vec<u8>> = (0..(rng.next_u32() % 8 + 1))
            .map(|_| {
                let mut m = random_bytes(&mut rng, 600);
                m.insert(0, rng.next_u32() as u8); // the type byte: never empty
                m
            })
            .collect();
        let mut stream = Vec::new();
        for m in &messages {
            stream.extend_from_slice(&(m.len() as u32).to_le_bytes());
            stream.extend_from_slice(m);
        }

        let mut inbox = Vec::new();
        let mut received = Vec::new();
        let mut at = 0;
        while at < stream.len() {
            let n = ((rng.next_u32() % 64) as usize + 1).min(stream.len() - at);
            inbox.extend_from_slice(&stream[at..at + n]);
            at += n;
            while let Some(m) = take_control_message(&mut inbox).expect("well-formed lengths") {
                received.push(m);
            }
        }
        assert_eq!(received, messages);
        assert!(inbox.is_empty());
    }

    // Garbage never panics, and a length past MAX_MSG_LEN is refused.
    for _ in 0..ITERATIONS {
        let mut inbox = random_bytes(&mut rng, 256);
        while let Ok(Some(_)) = take_control_message(&mut inbox) {}
    }
    let mut inbox = ((fvp_common::MAX_MSG_LEN + 1) as u32).to_le_bytes().to_vec();
    assert_eq!(take_control_message(&mut inbox), Err(fvp_common::MAX_MSG_LEN + 1));
}

// ---------- The video path ----------

#[test]
fn fuzz_frame_fec_roundtrip() {
    // What the engine sends (FrameFecEncoder: bulk, sliced or unprotected,
    // any size, any redundancy) is what a receiver rebuilds, also with a
    // data packet lost from each code word that has parity to spare.
    use streaming_engine::pipeline::{FecFrameReassembler, FrameFecEncoder};
    use streaming_engine::transport::rtp::RtpPacketizer;

    let mut rng = rand::thread_rng();
    let mut packetizer = RtpPacketizer::new(0x46565000);

    for frame_index in 0..150u32 {
        // Mostly P-frame sizes, sometimes IDR-sized (sliced), now and then
        // past what 15 slices can protect at any redundancy (unprotected;
        // still under the receiver's MAX_FRAME_SHARDS).
        let len = match rng.next_u32() % 20 {
            0 => 4_600_000 + (rng.next_u32() % 200_000) as usize,
            1 | 2 => (rng.next_u32() % 1_000_000) as usize,
            3..=8 => (rng.next_u32() % 300_000) as usize,
            _ => (rng.next_u32() % 20_000) as usize,
        };
        let mut frame = vec![0u8; len];
        rng.fill_bytes(&mut frame);
        let redundancy = (rng.next_u32() % 50 + 1) as f32 / 100.0;
        let slices = (rng.next_u32() % 14 + 2) as u8;
        let mut encoder = FrameFecEncoder::new(redundancy, !rng.next_u32().is_multiple_of(4), slices);
        let keyframe = rng.next_u32().is_multiple_of(2);

        let batches = encoder.encode(&frame, frame_index, frame_index * 1500, keyframe, &mut packetizer);
        if frame.is_empty() {
            assert!(batches.is_empty());
            continue;
        }

        let lose_one = rng.next_u32().is_multiple_of(2);
        let mut reassembler = FecFrameReassembler::new();
        let mut rebuilt = None;
        for batch in &batches {
            // A code word with parity can lose its first packet (data).
            let parity = batch.iter().filter(|p| is_parity(&p.data)).count();
            let skip = usize::from(lose_one && parity > 0);
            for packet in &batch[skip..] {
                if let Some(done) = reassembler.feed(&packet.data) {
                    assert!(rebuilt.is_none(), "frame {frame_index} came out twice");
                    rebuilt = Some(done);
                }
            }
        }
        let rebuilt = rebuilt.unwrap_or_else(|| panic!("frame {frame_index} ({len} B) never came out"));
        assert_eq!(rebuilt.frame_index, frame_index);
        assert_eq!(rebuilt.is_keyframe, keyframe);
        // Bulk frames come back zero-padded to whole shards.
        assert!(rebuilt.data.len() >= frame.len(), "{} < {}", rebuilt.data.len(), frame.len());
        assert_eq!(&rebuilt.data[..frame.len()], &frame[..], "frame {frame_index} ({len} B) differs");
        assert!(rebuilt.data[frame.len()..].iter().all(|&b| b == 0));
    }

    /// A parity shard: its FVP shard index is at or past the data count.
    fn is_parity(packet: &[u8]) -> bool {
        let u16_at = |at: usize| u16::from_le_bytes([packet[at], packet[at + 1]]);
        u16_at(16) >= u16_at(22)
    }
}

// ---------- Config ----------

#[test]
fn fuzz_config_parse_validate_no_panic() {
    use streaming_engine::config::AppConfig;

    let mut rng = rand::thread_rng();

    for _ in 0..ITERATIONS {
        let data = random_bytes(&mut rng, 4096);
        let Ok(text) = std::str::from_utf8(&data) else { continue };

        let mut config: AppConfig = match toml::from_str(text) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Must not panic
        let _ = config.validate();
    }
}
