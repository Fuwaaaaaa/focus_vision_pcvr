use crate::transport::fec::{FecDecoder, FecEncoder};
use crate::transport::rtp::{RtpPacket, RtpPacketizer};
use fvp_common::FEC_SHARD_SIZE;

/// Encode a frame into FEC-protected RTP packets ready for UDP transmission.
pub fn encode_frame_to_packets(
    frame_data: &[u8],
    frame_index: u32,
    timestamp_90khz: u32,
    is_keyframe: bool,
    fec_redundancy: f32,
    packetizer: &mut RtpPacketizer,
) -> Vec<RtpPacket> {
    // Step 1: Split frame into FEC shards
    let shard_size = FEC_SHARD_SIZE;
    let data_shards: Vec<Vec<u8>> = frame_data
        .chunks(shard_size)
        .map(|chunk| {
            let mut shard = chunk.to_vec();
            // Pad last shard to equal length
            shard.resize(shard_size, 0);
            shard
        })
        .collect();

    if data_shards.is_empty() {
        return vec![];
    }

    // Step 2: FEC encode (add parity shards)
    let fec = FecEncoder::new(fec_redundancy);
    let all_shards = match fec.encode(&data_shards) {
        Ok(shards) => shards,
        Err(e) => {
            log::warn!("FEC encode failed: {e}, sending without FEC");
            data_shards
        }
    };

    // Step 3: Each shard becomes an RTP packet payload
    let total_shards = all_shards.len();
    let mut packets = Vec::with_capacity(total_shards);

    for (i, shard) in all_shards.iter().enumerate() {
        let is_last = i == total_shards - 1;
        let seq = packetizer_next_seq(packetizer);

        let mut buf = Vec::with_capacity(12 + 8 + shard.len());

        // RTP header
        buf.push(0x80);
        let mpt = if is_last {
            0x80 | fvp_common::RTP_PT_H265
        } else {
            fvp_common::RTP_PT_H265
        };
        buf.push(mpt);
        buf.extend_from_slice(&seq.to_be_bytes());
        buf.extend_from_slice(&timestamp_90khz.to_be_bytes());
        buf.extend_from_slice(&0x42u32.to_be_bytes()); // SSRC

        // FVP header
        buf.extend_from_slice(&frame_index.to_le_bytes());
        buf.push(i as u8);
        buf.push(total_shards as u8);
        let flags: u16 = if is_keyframe { 1 } else { 0 };
        buf.extend_from_slice(&flags.to_le_bytes());

        buf.extend_from_slice(shard);

        packets.push(RtpPacket { data: buf });
    }

    packets
}

// Helper to get next sequence number from packetizer
fn packetizer_next_seq(_pkt: &mut RtpPacketizer) -> u16 {
    // We use a dummy packetize call to get the sequence, but this is hacky.
    // Better: expose sequence directly. For now, use the packetizer's internal counter.
    // Since we're building packets manually in the FEC path, we'll track sequence ourselves.
    static SEQ: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Decode FEC-protected RTP packets back into a frame.
/// `packets`: received RTP packets for one frame (some may be missing).
/// `data_shard_count`: number of original data shards.
/// Returns the reassembled frame data.
pub fn decode_packets_to_frame(
    packets: &[&[u8]],
    data_shard_count: usize,
    total_shard_count: usize,
    original_frame_len: usize,
) -> Result<Vec<u8>, String> {
    // Parse each packet to extract shard_index and payload
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; total_shard_count];

    for pkt in packets {
        if pkt.len() < 20 {
            continue;
        }
        let shard_index = pkt[16] as usize;
        let payload = &pkt[20..];
        if shard_index < total_shard_count {
            shards[shard_index] = Some(payload.to_vec());
        }
    }

    // Try FEC reconstruction
    let data_shards = FecDecoder::decode(&mut shards, data_shard_count)
        .map_err(|e| format!("FEC decode failed: {e}"))?;

    // Concatenate data shards and trim to original length
    let mut frame_data: Vec<u8> = data_shards.into_iter().flatten().collect();
    frame_data.truncate(original_frame_len);

    Ok(frame_data)
}
