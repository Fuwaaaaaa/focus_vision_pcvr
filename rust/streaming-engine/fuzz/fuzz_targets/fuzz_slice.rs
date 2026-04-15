#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use streaming_engine::transport::slice::SliceSplitter;
use streaming_engine::transport::fec::{FecEncoder, FecDecoder};

#[derive(Debug, Arbitrary)]
struct SliceInput {
    data: Vec<u8>,
    slice_count: u8,
    redundancy_pct: u8,
    drop_mask: Vec<bool>,
}

fuzz_target!(|input: SliceInput| {
    // Bound inputs
    if input.data.is_empty() || input.data.len() > 64 * 1024 {
        return;
    }
    let count = input.slice_count.clamp(1, 15);

    // Test SliceSplitter: split and reassemble
    let slices = SliceSplitter::split(&input.data, count);

    // Verify: concatenating slices produces original data
    let reassembled: Vec<u8> = slices.iter().flat_map(|s| s.iter().copied()).collect();
    assert_eq!(reassembled, input.data, "SliceSplitter roundtrip failed");

    // Verify: total slice count matches requested
    assert_eq!(slices.len(), count as usize);

    // Test FEC encode/decode per-slice (simulating the full slice FEC pipeline)
    let redundancy = (input.redundancy_pct as f32).clamp(5.0, 80.0) / 100.0;
    let shard_size = 1200usize;

    for (si, slice_data) in slices.iter().enumerate() {
        if slice_data.is_empty() {
            continue;
        }

        // Add u32 length prefix (matching pipeline.rs behavior)
        let original_len = slice_data.len() as u32;
        let mut prefixed = Vec::with_capacity(4 + slice_data.len());
        prefixed.extend_from_slice(&original_len.to_le_bytes());
        prefixed.extend_from_slice(slice_data);

        // Split into shards
        let data_shards: Vec<Vec<u8>> = prefixed
            .chunks(shard_size)
            .map(|chunk| {
                let mut shard = vec![0u8; shard_size];
                shard[..chunk.len()].copy_from_slice(chunk);
                shard
            })
            .collect();

        if data_shards.is_empty() || data_shards.len() > 200 {
            continue;
        }

        let data_count = data_shards.len();
        let mut encoder = FecEncoder::new(redundancy);
        let all_shards = match encoder.encode(data_shards) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Simulate loss
        let mut shards_with_loss: Vec<Option<Vec<u8>>> = all_shards
            .into_iter()
            .enumerate()
            .map(|(i, shard)| {
                if input.drop_mask.get(si * 50 + i).copied().unwrap_or(false) {
                    None
                } else {
                    Some(shard)
                }
            })
            .collect();

        let available = shards_with_loss.iter().filter(|s| s.is_some()).count();

        match FecDecoder::decode(&mut shards_with_loss, data_count) {
            Ok(recovered) => {
                if available >= data_count {
                    // Reconstruct prefixed data
                    let mut recovered_data: Vec<u8> = recovered.into_iter().flatten().collect();
                    // Extract length prefix
                    if recovered_data.len() >= 4 {
                        let len = u32::from_le_bytes([
                            recovered_data[0], recovered_data[1],
                            recovered_data[2], recovered_data[3],
                        ]);
                        if (len as usize) + 4 <= recovered_data.len() {
                            let payload = &recovered_data[4..4 + len as usize];
                            assert_eq!(payload, *slice_data,
                                "Slice {} data mismatch after FEC roundtrip", si);
                        }
                    }
                }
            }
            Err(_) => {}
        }
    }
});
