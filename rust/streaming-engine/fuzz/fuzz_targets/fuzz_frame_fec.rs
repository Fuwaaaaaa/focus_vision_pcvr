#![no_main]

//! The engine's video path end to end: FrameFecEncoder (bulk or sliced, any
//! redundancy and slice count) then FecFrameReassembler must give the frame
//! back, also with one data packet lost from each code word that has parity.
//! (The stable-toolchain counterpart is tests/fuzz_tests.rs.)

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use streaming_engine::pipeline::{FecFrameReassembler, FrameFecEncoder};
use streaming_engine::transport::rtp::RtpPacketizer;

#[derive(Debug, Arbitrary)]
struct FrameInput {
    frame: Vec<u8>,
    redundancy_pct: u8,
    slice_fec: bool,
    slice_count: u8,
    keyframe: bool,
    lose_first_packet: bool,
}

fuzz_target!(|input: FrameInput| {
    // Bounded so each run stays fast; the sliced path starts at 16 KB.
    if input.frame.is_empty() || input.frame.len() > 256 * 1024 {
        return;
    }
    let redundancy = f32::from(input.redundancy_pct.clamp(1, 60)) / 100.0;
    let mut encoder = FrameFecEncoder::new(redundancy, input.slice_fec, input.slice_count.clamp(2, 15));
    let mut packetizer = RtpPacketizer::new(0x46565000);
    let batches = encoder.encode(&input.frame, 7, 1500, input.keyframe, &mut packetizer);
    assert!(!batches.is_empty());

    let mut reassembler = FecFrameReassembler::new();
    let mut rebuilt = None;
    for batch in &batches {
        let parity = batch.iter().filter(|p| {
            let u16_at = |at: usize| u16::from_le_bytes([p.data[at], p.data[at + 1]]);
            u16_at(16) >= u16_at(22)
        }).count();
        let skip = usize::from(input.lose_first_packet && parity > 0);
        for packet in &batch[skip..] {
            if let Some(done) = reassembler.feed(&packet.data) {
                assert!(rebuilt.is_none(), "the frame came out twice");
                rebuilt = Some(done);
            }
        }
    }
    let rebuilt = rebuilt.expect("the frame never came out");
    assert_eq!(rebuilt.frame_index, 7);
    assert_eq!(rebuilt.is_keyframe, input.keyframe);
    // Bulk frames come back zero-padded to whole shards.
    assert_eq!(&rebuilt.data[..input.frame.len()], &input.frame[..]);
    assert!(rebuilt.data[input.frame.len()..].iter().all(|&b| b == 0));
});
