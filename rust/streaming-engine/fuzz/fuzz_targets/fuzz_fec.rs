#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use streaming_engine::transport::fec::{FecEncoder, FecDecoder};

#[derive(Debug, Arbitrary)]
struct FecInput {
    /// Raw data to split into shards
    data: Vec<u8>,
    /// Shard size (1-1200 bytes)
    shard_size: u8,
    /// Redundancy percentage (1-100)
    redundancy_pct: u8,
    /// Bitmask of which shards to drop (simulates packet loss)
    drop_mask: Vec<bool>,
}

fuzz_target!(|input: FecInput| {
    // Bound inputs to prevent OOM
    if input.data.is_empty() || input.data.len() > 32 * 1024 {
        return;
    }
    let shard_size = (input.shard_size as usize).max(1).min(1200);
    let redundancy = (input.redundancy_pct as f32).clamp(1.0, 100.0) / 100.0;

    // Split data into fixed-size shards
    let data_shards: Vec<Vec<u8>> = input.data
        .chunks(shard_size)
        .map(|chunk| {
            let mut shard = chunk.to_vec();
            // Pad last shard to uniform size
            shard.resize(shard_size, 0);
            shard
        })
        .collect();

    if data_shards.is_empty() || data_shards.len() > 255 {
        return;
    }

    let original_data: Vec<Vec<u8>> = data_shards.clone();
    let data_count = data_shards.len();

    // Encode with FEC
    let mut encoder = FecEncoder::new(redundancy);
    let all_shards = match encoder.encode(data_shards) {
        Ok(s) => s,
        Err(_) => return, // Invalid params, skip
    };

    let _total = all_shards.len();

    // Simulate packet loss using drop_mask
    let mut shards_with_loss: Vec<Option<Vec<u8>>> = all_shards
        .into_iter()
        .enumerate()
        .map(|(i, shard)| {
            if input.drop_mask.get(i).copied().unwrap_or(false) {
                None // dropped
            } else {
                Some(shard)
            }
        })
        .collect();

    let available = shards_with_loss.iter().filter(|s| s.is_some()).count();

    // Attempt decode
    match FecDecoder::decode(&mut shards_with_loss, data_count) {
        Ok(recovered) => {
            // If we had enough shards, recovered data must match original
            if available >= data_count {
                assert_eq!(recovered.len(), data_count);
                for (i, shard) in recovered.iter().enumerate() {
                    assert_eq!(shard, &original_data[i],
                        "Shard {} mismatch after FEC recovery", i);
                }
            }
        }
        Err(_) => {
            // Decode failure is acceptable when too many shards are lost
        }
    }
});
