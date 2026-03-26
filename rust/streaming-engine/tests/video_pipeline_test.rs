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
    let encoder = FecEncoder::new(0.2);

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

/// Large frame test: ensure packetization handles frames bigger than MTU
#[test]
fn test_large_frame_packetization() {
    let mut pkt = RtpPacketizer::new(1);
    // Simulate a 100KB encoded frame (realistic for H.265 at high bitrate)
    let frame = vec![0xAA; 100_000];
    let packets = pkt.packetize(&frame, 0, 0, true);

    // Each packet payload ≤ MTU - 20 bytes header
    let max_payload = fvp_common::MTU_SIZE - 12 - 8;
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
