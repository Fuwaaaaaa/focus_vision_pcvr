#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use streaming_engine::transport::rtp::RtpPacketizer;

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

    // Verify each packet has at least a valid RTP + FVP header (22 bytes)
    for pkt in &packets {
        assert!(pkt.data.len() >= 22, "Packet too small: {} bytes", pkt.data.len());
    }

    // Recycle buffers (exercises pool logic)
    packetizer.recycle(packets);
});
