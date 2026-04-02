use streaming_engine::transport::fec::{FecDecoder, FecEncoder};
use streaming_engine::transport::rtp::{RtpDepacketizer, RtpPacketizer};
use streaming_engine::transport::udp::{UdpReceiver, UdpSender};
use streaming_engine::video::test_pattern::generate_nv12_frame;

/// Full pipeline test: generate frame → RTP packetize → UDP send → receive → depacketize → verify
#[tokio::test]
async fn test_full_rtp_udp_pipeline() {
    let receiver = UdpReceiver::new("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let recv_addr = receiver.local_addr().unwrap();
    let sender = UdpSender::new(recv_addr).await.unwrap();

    // Generate a test frame (small for fast test)
    let original_frame = generate_nv12_frame(64, 64, 42);

    // Packetize
    let mut packetizer = RtpPacketizer::new(0xDEAD);
    let packets = packetizer.packetize(&original_frame, 0, 9000, true);
    assert!(!packets.is_empty(), "Should produce at least 1 packet");

    // Send via UDP
    sender.send_all(&packets).await.unwrap();

    // Receive and depacketize
    let mut depacketizer = RtpDepacketizer::new();
    let mut result = None;
    let mut buf = [0u8; 2000];

    for _ in 0..packets.len() {
        let (len, _) = receiver.recv(&mut buf).await.unwrap();
        if let Some(frame) = depacketizer.feed(&buf[..len]) {
            result = Some(frame);
        }
    }

    let frame = result.expect("Should have reassembled the frame");
    assert_eq!(frame.frame_index, 0);
    assert!(frame.is_keyframe);
    assert_eq!(frame.data, original_frame);
}

/// Test multiple frames through the pipeline
#[tokio::test]
async fn test_multi_frame_pipeline() {
    let receiver = UdpReceiver::new("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let recv_addr = receiver.local_addr().unwrap();
    let sender = UdpSender::new(recv_addr).await.unwrap();

    let mut packetizer = RtpPacketizer::new(0xBEEF);
    let mut depacketizer = RtpDepacketizer::new();
    let mut buf = [0u8; 2000];

    for frame_num in 0..5u64 {
        let original = generate_nv12_frame(32, 32, frame_num);
        let packets = packetizer.packetize(&original, frame_num as u32, frame_num as u32 * 3000, false);

        sender.send_all(&packets).await.unwrap();

        let mut result = None;
        for _ in 0..packets.len() {
            let (len, _) = receiver.recv(&mut buf).await.unwrap();
            result = depacketizer.feed(&buf[..len]).or(result);
        }

        let frame = result.expect("Frame should be reassembled");
        assert_eq!(frame.frame_index, frame_num as u32);
        assert_eq!(frame.data, original);
    }
}

/// FEC test: encode with redundancy, drop some shards, verify recovery
#[test]
fn test_fec_pipeline_with_packet_loss() {
    let mut encoder = FecEncoder::new(0.2);

    // Create a "frame" of 5000 bytes
    let frame_data: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
    let shard_size = fvp_common::FEC_SHARD_SIZE;

    // Split into data shards
    let data_shards: Vec<Vec<u8>> = frame_data
        .chunks(shard_size)
        .map(|c| {
            let mut s = c.to_vec();
            s.resize(shard_size, 0);
            s
        })
        .collect();
    let data_count = data_shards.len();

    // FEC encode
    let all_shards = encoder.encode(&data_shards).unwrap();
    let total = all_shards.len();
    let parity_count = total - data_count;

    // Simulate packet loss: drop up to parity_count shards
    let mut received: Vec<Option<Vec<u8>>> = all_shards.into_iter().map(Some).collect();
    // Drop the first parity_count data shards
    for i in 0..parity_count {
        received[i] = None;
    }

    // FEC decode
    let recovered = FecDecoder::decode(&mut received, data_count).unwrap();
    let mut recovered_frame: Vec<u8> = recovered.into_iter().flatten().collect();
    recovered_frame.truncate(frame_data.len());
    assert_eq!(recovered_frame, frame_data);
}

/// IDR flag propagation: verify is_keyframe flag reaches FVP header in RTP packets
#[test]
fn test_idr_flag_in_rtp_packets() {
    use streaming_engine::pipeline::encode_frame_to_packets;

    let nal_data = vec![0xAB; 500]; // Mock NAL
    let mut packetizer = RtpPacketizer::new(0x1234);

    // IDR frame
    let idr_packets = encode_frame_to_packets(&nal_data, 0, 9000, true, 0.2, &mut packetizer);
    assert!(!idr_packets.is_empty());
    // FVP header flags at offset 20 (12 RTP + 4 frame_index + 2 shard_idx + 2 shard_count)
    let flags = u16::from_le_bytes([idr_packets[0].data[20], idr_packets[0].data[21]]);
    assert_eq!(flags & 0x01, 1, "IDR flag should be set");

    // Non-IDR frame
    let non_idr_packets = encode_frame_to_packets(&nal_data, 1, 18000, false, 0.2, &mut packetizer);
    let flags = u16::from_le_bytes([non_idr_packets[0].data[20], non_idr_packets[0].data[21]]);
    assert_eq!(flags & 0x01, 0, "IDR flag should NOT be set");
}

/// NAL data → RTP+FEC → reconstruct → verify NAL data preserved
#[test]
fn test_nal_to_rtp_fec_roundtrip() {
    use streaming_engine::pipeline::{encode_frame_to_packets, decode_packets_to_frame};

    // Simulate H.265 NAL: start code + IDR header + payload
    let mut nal_data = vec![0x00, 0x00, 0x00, 0x01, 0x26, 0x01]; // IDR_W_RADL
    nal_data.extend(vec![0xCD; 3000]); // Payload

    let mut packetizer = RtpPacketizer::new(0x5678);
    let packets = encode_frame_to_packets(&nal_data, 42, 90000, true, 0.2, &mut packetizer);
    assert!(packets.len() > 1, "NAL should span multiple RTP packets");

    // Extract shard counts from first packet's FVP header (u16 LE at offset 18)
    let total_shards = u16::from_le_bytes([packets[0].data[18], packets[0].data[19]]) as usize;
    let shard_size = fvp_common::FEC_SHARD_SIZE;
    let data_shard_count = (nal_data.len() + shard_size - 1) / shard_size;

    // Reconstruct: collect all packet refs
    let pkt_refs: Vec<&[u8]> = packets.iter().map(|p| p.data.as_slice()).collect();
    let recovered = decode_packets_to_frame(&pkt_refs, data_shard_count, total_shards, nal_data.len());
    assert!(recovered.is_ok(), "Decode should succeed with all packets");
    assert_eq!(recovered.unwrap(), nal_data, "NAL data should be preserved");
}

/// NAL data + FEC recovery: drop packets within parity budget, still recover
#[test]
fn test_nal_fec_recovery_with_loss() {
    use streaming_engine::pipeline::{encode_frame_to_packets, decode_packets_to_frame};

    let mut nal_data = vec![0x00, 0x00, 0x00, 0x01, 0x02, 0x01]; // TRAIL_R
    nal_data.extend(vec![0xEF; 5000]);

    let mut packetizer = RtpPacketizer::new(0x9ABC);
    let packets = encode_frame_to_packets(&nal_data, 7, 63000, false, 0.2, &mut packetizer);

    let total_shards = u16::from_le_bytes([packets[0].data[18], packets[0].data[19]]) as usize;
    let shard_size = fvp_common::FEC_SHARD_SIZE;
    let data_shard_count = (nal_data.len() + shard_size - 1) / shard_size;
    let parity_count = total_shards - data_shard_count;

    // Drop up to parity_count packets (should still recover)
    let mut surviving: Vec<&[u8]> = Vec::new();
    for (i, pkt) in packets.iter().enumerate() {
        if i >= parity_count { // skip first parity_count packets
            surviving.push(pkt.data.as_slice());
        }
    }

    let recovered = decode_packets_to_frame(&surviving, data_shard_count, total_shards, nal_data.len());
    assert!(recovered.is_ok(), "Should recover with FEC");
    assert_eq!(recovered.unwrap(), nal_data);
}

/// Large frame test: ensure packetization handles frames bigger than MTU
#[test]
fn test_large_frame_packetization() {
    let mut pkt = RtpPacketizer::new(1);
    // Simulate a 100KB encoded frame (realistic for H.265 at high bitrate)
    let frame = vec![0xAA; 100_000];
    let packets = pkt.packetize(&frame, 0, 0, true);

    assert!(packets.len() > 1);

    // Verify round-trip
    let mut depkt = RtpDepacketizer::new();
    let mut result = None;
    for p in &packets {
        result = depkt.feed(&p.data).or(result);
    }
    let frame_out = result.unwrap();
    assert_eq!(frame_out.data, frame);
}
