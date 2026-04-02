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
    let mut fec = FecEncoder::new(fec_redundancy);
    encode_frame_to_packets_with_fec(
        frame_data, frame_index, timestamp_90khz, is_keyframe, &mut fec, packetizer,
    )
}

/// Encode a frame using a reusable FecEncoder (avoids per-frame RS init).
pub fn encode_frame_to_packets_with_fec(
    frame_data: &[u8],
    frame_index: u32,
    timestamp_90khz: u32,
    is_keyframe: bool,
    fec: &mut FecEncoder,
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

    // Step 2: FEC encode (add parity shards, RS instance cached in FecEncoder)
    let all_shards = match fec.encode(&data_shards) {
        Ok(shards) => shards,
        Err(e) => {
            log::warn!("FEC encode failed: {e}, sending without FEC");
            data_shards
        }
    };

    // Step 3: Each shard becomes an RTP packet payload
    let total_shards = all_shards.len();
    if total_shards > u16::MAX as usize {
        log::error!("Frame too large: {} shards exceeds u16 max. Dropping frame.", total_shards);
        return vec![];
    }
    let mut packets = Vec::with_capacity(total_shards);

    for (i, shard) in all_shards.iter().enumerate() {
        let is_last = i == total_shards - 1;
        let seq = packetizer.next_sequence();

        let mut buf = Vec::with_capacity(12 + 10 + shard.len());

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

        // FVP header (10 bytes) — shard fields are u16 to support large keyframes
        buf.extend_from_slice(&frame_index.to_le_bytes());
        buf.extend_from_slice(&(i as u16).to_le_bytes());            // shard_index
        buf.extend_from_slice(&(total_shards as u16).to_le_bytes()); // shard_count
        let flags: u16 = if is_keyframe { 1 } else { 0 };
        buf.extend_from_slice(&flags.to_le_bytes());

        buf.extend_from_slice(shard);

        packets.push(RtpPacket { data: buf });
    }

    packets
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
    // Sanity check: reject absurd shard counts
    const MAX_SHARDS: usize = 4096;
    if total_shard_count == 0 || total_shard_count > MAX_SHARDS || data_shard_count > total_shard_count {
        return Err("Invalid shard counts".into());
    }

    // Parse each packet to extract shard_index and payload
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; total_shard_count];

    for pkt in packets {
        if pkt.len() < 22 {
            continue;
        }
        let shard_index = u16::from_le_bytes([pkt[16], pkt[17]]) as usize;
        let payload = &pkt[22..];
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
