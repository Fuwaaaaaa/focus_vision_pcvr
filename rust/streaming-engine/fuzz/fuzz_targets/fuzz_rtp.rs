#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use streaming_engine::pipeline::FecFrameReassembler;
use streaming_engine::transport::rtp::{RtpDepacketizer, RtpPacketizer};

#[derive(Debug, Arbitrary)]
struct RtpInput {
    frame_data: Vec<u8>,
    frame_index: u32,
    timestamp: u32,
    is_keyframe: bool,
}

fuzz_target!(|input: RtpInput| {
    // Limit frame size to prevent OOM
    if input.frame_data.len() > 128 * 1024 {
        return;
    }

    let mut packetizer = RtpPacketizer::new(0x12345678);

    let packets = packetizer.packetize(
        &input.frame_data,
        input.frame_index,
        input.timestamp,
        input.is_keyframe,
    );

    // Verify each packet has at least a valid RTP + FVP header (24 bytes)
    for pkt in &packets {
        assert!(pkt.data.len() >= fvp_common::PACKET_HEADER_LEN, "Packet too small: {} bytes", pkt.data.len());
    }

    // Receiver side: the raw fuzz bytes as an untrusted packet (forged
    // shard / data_shard_count fields) must never panic or over-allocate.
    let mut reassembler = FecFrameReassembler::new();
    let _ = reassembler.feed(&input.frame_data);
    let _ = RtpDepacketizer::new().feed(&input.frame_data);

    // Recycle buffers (exercises pool logic)
    packetizer.recycle(packets);
});
