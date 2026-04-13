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
        let is_keyframe = rng.next_u32() % 2 == 0;

        let packets = packetizer.packetize(&data, frame_index, timestamp, is_keyframe);

        for pkt in &packets {
            assert!(pkt.data.len() >= 22, "Packet too small: {}", pkt.data.len());
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
        match FecDecoder::decode(&mut shards, data_count) {
            Ok(recovered) => {
                if available >= data_count {
                    assert_eq!(recovered.len(), data_count);
                    for (i, shard) in recovered.iter().enumerate() {
                        assert_eq!(shard, &original[i], "Shard {} mismatch", i);
                    }
                }
            }
            Err(_) => {} // acceptable
        }
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
