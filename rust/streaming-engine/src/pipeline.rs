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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::rtp::RtpPacketizer;

    /// Helper: create a packetizer with default SSRC.
    fn make_packetizer() -> RtpPacketizer {
        RtpPacketizer::new(0x42)
    }

    #[test]
    fn test_encode_empty_frame_returns_no_packets() {
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&[], 0, 0, false, 0.2, &mut pkt);
        assert!(packets.is_empty(), "Empty frame should produce zero packets");
    }

    #[test]
    fn test_encode_small_frame_single_shard() {
        // A frame smaller than FEC_SHARD_SIZE should produce a small number of
        // packets (data shards + parity shards). With 0.2 redundancy and 1 data
        // shard we get ceil(1*0.2)=max(1,1)=1 parity, so 2 total packets.
        let frame = vec![0xAB; 100];
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&frame, 1, 9000, false, 0.2, &mut pkt);

        // 1 data shard + 1 parity shard = 2 packets
        assert_eq!(packets.len(), 2);

        // Each packet should be: 12 (RTP) + 10 (FVP) + FEC_SHARD_SIZE bytes
        for p in &packets {
            assert_eq!(p.data.len(), 12 + 10 + FEC_SHARD_SIZE);
        }
    }

    #[test]
    fn test_rtp_header_fields() {
        let frame = vec![0xFF; 50];
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&frame, 42, 12345, false, 0.2, &mut pkt);
        assert!(!packets.is_empty());

        let data = &packets[0].data;
        // Byte 0: V=2, P=0, X=0, CC=0 => 0x80
        assert_eq!(data[0], 0x80);

        // Bytes 4..8: timestamp in big-endian
        let ts = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!(ts, 12345);

        // Bytes 8..12: SSRC = 0x42 in big-endian
        let ssrc = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        assert_eq!(ssrc, 0x42);
    }

    #[test]
    fn test_fvp_header_frame_index_and_shard_fields() {
        let frame = vec![0x11; FEC_SHARD_SIZE * 3 + 10]; // 4 data shards
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&frame, 99, 0, false, 0.2, &mut pkt);

        // 4 data shards, parity = ceil(4*0.2) = max(1,1) = 1 => 5 total
        assert_eq!(packets.len(), 5);

        for (i, p) in packets.iter().enumerate() {
            let d = &p.data;
            // FVP header starts at byte 12
            let frame_idx = u32::from_le_bytes([d[12], d[13], d[14], d[15]]);
            assert_eq!(frame_idx, 99, "frame_index mismatch at shard {i}");

            let shard_index = u16::from_le_bytes([d[16], d[17]]) as usize;
            assert_eq!(shard_index, i, "shard_index mismatch at shard {i}");

            let shard_count = u16::from_le_bytes([d[18], d[19]]) as usize;
            assert_eq!(shard_count, 5, "shard_count mismatch at shard {i}");
        }
    }

    #[test]
    fn test_keyframe_flag_set_in_fvp_header() {
        let frame = vec![0xCC; 100];
        let mut pkt = make_packetizer();

        // Keyframe
        let kf_packets = encode_frame_to_packets(&frame, 0, 0, true, 0.2, &mut pkt);
        for p in &kf_packets {
            let flags = u16::from_le_bytes([p.data[20], p.data[21]]);
            assert_eq!(flags & 1, 1, "Keyframe flag should be set");
        }

        // Non-keyframe
        let nkf_packets = encode_frame_to_packets(&frame, 1, 0, false, 0.2, &mut pkt);
        for p in &nkf_packets {
            let flags = u16::from_le_bytes([p.data[20], p.data[21]]);
            assert_eq!(flags & 1, 0, "Keyframe flag should be clear");
        }
    }

    #[test]
    fn test_marker_bit_only_on_last_packet() {
        let frame = vec![0xDD; FEC_SHARD_SIZE * 2 + 1]; // 3 data shards
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&frame, 0, 0, false, 0.2, &mut pkt);
        assert!(packets.len() >= 3);

        for (i, p) in packets.iter().enumerate() {
            let marker = p.data[1] & 0x80;
            if i == packets.len() - 1 {
                assert_ne!(marker, 0, "Last packet must have marker bit set");
            } else {
                assert_eq!(marker, 0, "Non-last packet must not have marker bit");
            }
        }
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = vec![0xEE; 5000];
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&original, 7, 45000, true, 0.2, &mut pkt);

        // Determine data/total shard counts from the first packet's FVP header
        let total_shard_count =
            u16::from_le_bytes([packets[0].data[18], packets[0].data[19]]) as usize;
        let data_shard_count =
            (original.len() + FEC_SHARD_SIZE - 1) / FEC_SHARD_SIZE;

        let pkt_refs: Vec<&[u8]> = packets.iter().map(|p| p.data.as_slice()).collect();
        let decoded =
            decode_packets_to_frame(&pkt_refs, data_shard_count, total_shard_count, original.len())
                .expect("decode should succeed");

        assert_eq!(decoded, original);
    }

    #[test]
    fn test_decode_recovers_with_lost_packets() {
        // Encode with enough redundancy to lose some shards
        let original = vec![0xAA; FEC_SHARD_SIZE * 4]; // exactly 4 data shards
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&original, 0, 0, false, 0.5, &mut pkt);

        let total_shard_count =
            u16::from_le_bytes([packets[0].data[18], packets[0].data[19]]) as usize;
        let data_shard_count = 4;
        // 0.5 redundancy on 4 data => ceil(2) = 2 parity, total = 6
        assert_eq!(total_shard_count, 6);

        // Drop 2 packets (within parity budget)
        let surviving: Vec<&[u8]> = packets
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 1 && *i != 3)
            .map(|(_, p)| p.data.as_slice())
            .collect();

        let decoded = decode_packets_to_frame(
            &surviving,
            data_shard_count,
            total_shard_count,
            original.len(),
        )
        .expect("FEC should recover 2 lost shards with 50% redundancy");

        assert_eq!(decoded, original);
    }

    #[test]
    fn test_decode_rejects_invalid_shard_counts() {
        // total = 0
        assert!(decode_packets_to_frame(&[], 0, 0, 100).is_err());
        // data > total
        assert!(decode_packets_to_frame(&[], 5, 3, 100).is_err());
        // total exceeds MAX_SHARDS (4096)
        assert!(decode_packets_to_frame(&[], 1, 5000, 100).is_err());
    }

    #[test]
    fn test_decode_skips_undersized_packets() {
        // Packets smaller than 22 bytes should be silently ignored
        let tiny: Vec<u8> = vec![0; 10];
        let pkt_refs: Vec<&[u8]> = vec![tiny.as_slice()];
        // This won't reconstruct anything, but it should not panic.
        // With 2 data, 1 parity = total 3 but no valid shards => FEC fails.
        let result = decode_packets_to_frame(&pkt_refs, 2, 3, 100);
        assert!(result.is_err());
    }
}
