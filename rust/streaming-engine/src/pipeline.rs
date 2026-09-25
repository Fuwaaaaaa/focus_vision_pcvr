use crate::transport::fec::{FecDecoder, FecEncoder};
use crate::transport::rtp::{read_fvp_header, ReassembledFrame, RtpPacket, RtpPacketizer};
use crate::transport::slice::SliceSplitter;
use fvp_common::protocol::fvp_flags;
use fvp_common::{FEC_SHARD_SIZE, MAX_FRAME_SHARDS, PACKET_HEADER_LEN};

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
            // Pre-allocate exact size, copy data, zero-pad remainder
            let mut shard = vec![0u8; shard_size];
            shard[..chunk.len()].copy_from_slice(chunk);
            shard
        })
        .collect();

    if data_shards.is_empty() {
        return vec![];
    }
    let data_count = data_shards.len();

    // Step 2: FEC encode (add parity shards, RS instance cached in FecEncoder)
    // data_shards ownership is moved into encode() to avoid cloning.
    let all_shards = match fec.encode(data_shards) {
        Ok(shards) => shards,
        Err(e) => {
            log::warn!("FEC encode failed: {e}, sending without FEC");
            // Rebuild minimal shards for fallback (rare error path). The
            // header still reports data_count == total, so the receiver
            // knows there is no parity to wait for.
            frame_data
                .chunks(shard_size)
                .map(|chunk| {
                    let mut shard = chunk.to_vec();
                    shard.resize(shard_size, 0);
                    shard
                })
                .collect()
        }
    };

    // Step 3: Each shard becomes an RTP packet payload.
    let flags = fvp_flags::encode_simple(is_keyframe);
    shards_to_packets(&all_shards, data_count, frame_index, timestamp_90khz, flags, packetizer)
}

/// Wrap each shard in RTP + FVP headers. `data_count` is written into every
/// packet's `data_shard_count` so the receiver never has to guess the
/// data/parity split — adaptive FEC changes it from frame to frame.
/// Buffer pool in packetizer avoids per-frame allocation after the first frame.
fn shards_to_packets(
    all_shards: &[Vec<u8>],
    data_count: usize,
    frame_index: u32,
    timestamp_90khz: u32,
    flags: u16,
    packetizer: &mut RtpPacketizer,
) -> Vec<RtpPacket> {
    let total_shards = all_shards.len();
    if total_shards > u16::MAX as usize {
        log::error!("Frame too large: {} shards exceeds u16 max. Dropping frame.", total_shards);
        return vec![];
    }
    debug_assert!(data_count > 0 && data_count <= total_shards);
    let mut packets = Vec::with_capacity(total_shards);

    for (i, shard) in all_shards.iter().enumerate() {
        let is_last = i == total_shards - 1;
        let seq = packetizer.next_sequence();

        let mut buf = packetizer.take_buf(PACKET_HEADER_LEN + shard.len());

        crate::transport::rtp::write_rtp_header(
            &mut buf, fvp_common::RTP_PT_H265, is_last, seq, timestamp_90khz, 0x42,
        );
        crate::transport::rtp::write_fvp_header(
            &mut buf, frame_index, i as u16, total_shards as u16, flags, data_count as u16,
        );

        buf.extend_from_slice(shard);

        packets.push(RtpPacket { data: buf });
    }

    packets
}

/// Minimum frame size for slice-based FEC to be beneficial.
/// Below this threshold, RS encoding is already fast enough that slicing adds overhead.
pub const MIN_SLICE_SIZE: usize = 16_384; // 16KB

/// Encode a frame using slice-based FEC: split into N slices, RS-encode each independently.
/// Each slice's packets are returned as a separate Vec so the caller can send progressively.
/// The first shard of each slice contains a u32 length prefix for the original slice data.
pub fn encode_frame_sliced(
    frame_data: &[u8],
    frame_index: u32,
    timestamp_90khz: u32,
    is_keyframe: bool,
    slice_count: u8,
    fec_encoders: &mut [FecEncoder],
    packetizer: &mut RtpPacketizer,
) -> Vec<Vec<RtpPacket>> {
    let slices = SliceSplitter::split(frame_data, slice_count);
    let mut all_packets = Vec::with_capacity(slices.len());

    for (slice_idx, slice_data) in slices.iter().enumerate() {
        if slice_data.is_empty() {
            all_packets.push(vec![]);
            continue;
        }

        // Prepend u32 length prefix to the slice data so the decoder can truncate after RS
        let original_len = slice_data.len() as u32;
        let mut prefixed = Vec::with_capacity(4 + slice_data.len());
        prefixed.extend_from_slice(&original_len.to_le_bytes());
        prefixed.extend_from_slice(slice_data);

        let shard_size = FEC_SHARD_SIZE;
        let data_shards: Vec<Vec<u8>> = prefixed
            .chunks(shard_size)
            .map(|chunk| {
                let mut shard = vec![0u8; shard_size];
                shard[..chunk.len()].copy_from_slice(chunk);
                shard
            })
            .collect();

        if data_shards.is_empty() {
            all_packets.push(vec![]);
            continue;
        }

        // Check RS shard limit: if per-slice data shards > 200, caller should use bulk FEC
        if data_shards.len() > 200 {
            log::warn!("Slice {} has {} data shards (>200), skipping slice FEC", slice_idx, data_shards.len());
            all_packets.push(vec![]);
            continue;
        }

        let data_count = data_shards.len();
        let fec = &mut fec_encoders[slice_idx];
        let all_shards = match fec.encode(data_shards) {
            Ok(shards) => shards,
            Err(e) => {
                log::warn!("Slice {} FEC encode failed: {e}", slice_idx);
                all_packets.push(vec![]);
                continue;
            }
        };

        let flags = fvp_flags::encode(is_keyframe, slice_idx as u8, slice_count, 0);
        all_packets.push(shards_to_packets(
            &all_shards, data_count, frame_index, timestamp_90khz, flags, packetizer,
        ));
    }

    all_packets
}

/// Decode FEC-protected RTP packets back into a frame.
/// `packets`: received RTP packets for one frame (some may be missing).
/// `data_shard_count`: number of original data shards.
/// Returns the reassembled frame data.
///
/// Low-level helper for callers that already know the shard counts; live
/// receivers should use [`FecFrameReassembler`], which reads them from each
/// packet's FVP header.
pub fn decode_packets_to_frame(
    packets: &[&[u8]],
    data_shard_count: usize,
    total_shard_count: usize,
    original_frame_len: usize,
) -> Result<Vec<u8>, String> {
    // Sanity check: reject absurd shard counts
    if total_shard_count == 0
        || total_shard_count > MAX_FRAME_SHARDS
        || data_shard_count == 0
        || data_shard_count > total_shard_count
    {
        return Err("Invalid shard counts".into());
    }

    // Parse each packet to extract shard_index and payload
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; total_shard_count];

    for pkt in packets {
        let Some(hdr) = read_fvp_header(pkt) else { continue };
        let shard_index = hdr.shard_index as usize;
        let payload = &pkt[PACKET_HEADER_LEN..];
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

/// Receiver-side FEC reassembly driven entirely by the FVP header.
///
/// Mirrors the Android client's `FecFrameDecoder` / `SlicedFecFrameDecoder`:
/// every packet carries `shard_count` and `data_shard_count`, so frames are
/// rebuilt correctly whatever parity ratio adaptive FEC chose for them. A
/// frame (or each slice of a sliced frame) completes as soon as
/// `data_shard_count` shards have arrived; missing data shards are
/// Reed-Solomon reconstructed from parity. Late parity packets for a frame
/// that was already emitted are ignored, so each frame is produced once.
///
/// Bulk frames come back zero-padded to a whole number of shards (the bulk
/// path carries no length), exactly as the client hands them to its video
/// decoder. Sliced frames have each slice's u32 length prefix stripped.
#[derive(Default)]
pub struct FecFrameReassembler {
    frame: Option<PendingFrame>,
}

struct PendingFrame {
    frame_index: u32,
    is_keyframe: bool,
    /// 0 = bulk (one RS group); otherwise the number of independently
    /// FEC-coded slices, each with its own shard numbering.
    slice_count: u8,
    groups: Vec<ShardGroup>,
    delivered: bool,
}

/// One Reed-Solomon group: the whole frame (bulk) or one slice.
#[derive(Default)]
struct ShardGroup {
    shard_count: usize,
    data_shard_count: usize,
    shard_len: usize,
    shards: Vec<Option<Vec<u8>>>,
    received: usize,
    decoded: Option<Vec<u8>>,
    failed: bool,
}

impl ShardGroup {
    /// Store one shard. Returns false for packets that do not fit the group:
    /// counts that disagree with its first packet, a payload size different
    /// from the other shards, or a duplicate index.
    fn accept(&mut self, shard_index: usize, shard_count: usize, data_count: usize, payload: &[u8]) -> bool {
        if self.shard_count == 0 {
            self.shard_count = shard_count;
            self.data_shard_count = data_count;
            self.shard_len = payload.len();
            self.shards = vec![None; shard_count];
        } else if self.shard_count != shard_count
            || self.data_shard_count != data_count
            || self.shard_len != payload.len()
        {
            return false;
        }
        if self.shards[shard_index].is_some() {
            return false;
        }
        self.shards[shard_index] = Some(payload.to_vec());
        self.received += 1;
        true
    }

    /// Rebuild the group's data once `data_shard_count` shards are present:
    /// the concatenated data shards, with the slice length prefix stripped
    /// when `has_len_prefix`. None if RS fails or the prefix is out of range.
    fn reconstruct(&mut self, has_len_prefix: bool) -> Option<Vec<u8>> {
        let d = self.data_shard_count;
        let data_shards: Vec<Vec<u8>> = if self.shards[..d].iter().all(Option::is_some) {
            self.shards[..d].iter_mut().map(|s| s.take().unwrap_or_default()).collect()
        } else {
            FecDecoder::decode(&mut self.shards, d).ok()?
        };
        self.shards = Vec::new();
        let mut data: Vec<u8> = data_shards.into_iter().flatten().collect();
        if has_len_prefix {
            if data.len() < 4 {
                return None;
            }
            let len = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
            if len > data.len() - 4 {
                return None;
            }
            data.truncate(4 + len);
            data.drain(..4);
        }
        Some(data)
    }
}

impl FecFrameReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one received packet. Returns the frame once it is complete.
    /// Malformed or inconsistent packets are dropped.
    pub fn feed(&mut self, packet: &[u8]) -> Option<ReassembledFrame> {
        let hdr = read_fvp_header(packet)?;
        if !hdr.is_valid() {
            return None;
        }
        let payload = &packet[PACKET_HEADER_LEN..];
        if payload.is_empty() {
            return None;
        }
        let slice_count = fvp_flags::slice_count(hdr.flags);
        let slice_index = fvp_flags::slice_index(hdr.flags);
        if slice_count > 0 && slice_index >= slice_count {
            return None;
        }

        // A new frame index abandons whatever the previous frame had not
        // completed (the client does the same, then requests an IDR).
        if self.frame.as_ref().map(|f| f.frame_index) != Some(hdr.frame_index) {
            self.frame = Some(PendingFrame {
                frame_index: hdr.frame_index,
                is_keyframe: hdr.is_keyframe(),
                slice_count,
                groups: (0..slice_count.max(1)).map(|_| ShardGroup::default()).collect(),
                delivered: false,
            });
        }
        let frame = self.frame.as_mut()?;
        if frame.delivered || frame.slice_count != slice_count {
            return None;
        }

        let group_idx = if slice_count == 0 { 0 } else { slice_index as usize };
        let group = &mut frame.groups[group_idx];
        if group.decoded.is_some() || group.failed {
            return None;
        }
        if !group.accept(
            hdr.shard_index as usize,
            hdr.shard_count as usize,
            hdr.data_shard_count as usize,
            payload,
        ) {
            return None;
        }
        if group.received >= group.data_shard_count {
            match group.reconstruct(slice_count > 0) {
                Some(data) => group.decoded = Some(data),
                None => {
                    group.failed = true;
                    return None;
                }
            }
        }

        if frame.groups.iter().all(|g| g.decoded.is_some()) {
            frame.delivered = true;
            let data = frame
                .groups
                .iter_mut()
                .flat_map(|g| g.decoded.take().unwrap_or_default())
                .collect();
            return Some(ReassembledFrame {
                frame_index: frame.frame_index,
                is_keyframe: frame.is_keyframe,
                data,
            });
        }
        None
    }
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

        // Each packet should be: 12 (RTP) + 12 (FVP) + FEC_SHARD_SIZE bytes
        for p in &packets {
            assert_eq!(p.data.len(), PACKET_HEADER_LEN + FEC_SHARD_SIZE);
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
        let data_shard_count = original.len().div_ceil(FEC_SHARD_SIZE);

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

    // --- Slice FEC tests ---

    #[test]
    fn test_sliced_fec_encode_4_slices() {
        let frame = vec![0xAB; 20_000]; // 20KB > MIN_SLICE_SIZE (16KB)
        let mut pkt = make_packetizer();
        let mut encoders: Vec<FecEncoder> = (0..4).map(|_| FecEncoder::new(0.2)).collect();

        let batches = encode_frame_sliced(&frame, 1, 9000, true, 4, &mut encoders, &mut pkt);
        assert_eq!(batches.len(), 4, "Should produce 4 slice batches");

        // Each batch should have packets
        for (i, batch) in batches.iter().enumerate() {
            assert!(!batch.is_empty(), "Slice {} should have packets", i);

            // Check fvp_flags on first packet of each batch
            let flags = u16::from_le_bytes([batch[0].data[20], batch[0].data[21]]);
            let si = fvp_common::protocol::fvp_flags::slice_index(flags);
            let sc = fvp_common::protocol::fvp_flags::slice_count(flags);
            assert_eq!(si, i as u8, "slice_index mismatch");
            assert_eq!(sc, 4, "slice_count should be 4");
            assert!(fvp_common::protocol::fvp_flags::is_keyframe(flags));
        }
    }

    #[test]
    fn test_sliced_fec_backward_compat_bulk_path() {
        // Frames below MIN_SLICE_SIZE should use bulk FEC (tested via encode_frame_to_packets)
        let frame = vec![0xCC; 1000]; // 1KB, well below 16KB threshold
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&frame, 0, 0, false, 0.2, &mut pkt);
        assert!(!packets.is_empty());

        // Verify flags use encode_simple (slice_index=0, slice_count=0)
        let flags = u16::from_le_bytes([packets[0].data[20], packets[0].data[21]]);
        assert_eq!(fvp_common::protocol::fvp_flags::slice_index(flags), 0);
        assert_eq!(fvp_common::protocol::fvp_flags::slice_count(flags), 0);
    }

    #[test]
    fn test_sliced_fec_payload_len_prefix() {
        let frame = vec![0xDD; 20_000];
        let mut pkt = make_packetizer();
        let mut encoders: Vec<FecEncoder> = (0..4).map(|_| FecEncoder::new(0.2)).collect();

        let batches = encode_frame_sliced(&frame, 0, 0, false, 4, &mut encoders, &mut pkt);

        // Each slice's first data shard should start with u32 length prefix
        for (i, batch) in batches.iter().enumerate() {
            if batch.is_empty() { continue; }
            // Extract payload from first packet (after 12B RTP + 12B FVP header)
            let payload = &batch[0].data[PACKET_HEADER_LEN..];
            let prefix_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            // Each slice of 20000/4 = 5000 bytes
            assert_eq!(prefix_len, 5000, "Slice {} payload len prefix mismatch", i);
        }
    }

    #[test]
    fn test_sliced_fec_small_frame_skips_empty_slices() {
        // A 3-byte frame split into 4 slices: [1B, 1B, 1B, 0B]
        // The 0-byte slice should produce empty batch
        let frame = vec![1, 2, 3];
        let mut pkt = make_packetizer();
        let mut encoders: Vec<FecEncoder> = (0..4).map(|_| FecEncoder::new(0.2)).collect();

        let batches = encode_frame_sliced(&frame, 0, 0, false, 4, &mut encoders, &mut pkt);
        assert_eq!(batches.len(), 4);
        // First 3 slices have data, 4th is empty
        assert!(!batches[0].is_empty());
        assert!(!batches[1].is_empty());
        assert!(!batches[2].is_empty());
        assert!(batches[3].is_empty());
    }

    #[test]
    fn test_sliced_fec_total_packet_count() {
        // With slice FEC, total packets should be roughly the same as bulk FEC
        // (each slice gets its own parity shards)
        let frame = vec![0xEE; 24_000]; // 24KB
        let mut pkt1 = make_packetizer();
        let mut pkt2 = make_packetizer();
        let mut bulk_enc = FecEncoder::new(0.2);
        let mut slice_encs: Vec<FecEncoder> = (0..4).map(|_| FecEncoder::new(0.2)).collect();

        let bulk_packets = encode_frame_to_packets_with_fec(
            &frame, 0, 0, false, &mut bulk_enc, &mut pkt1,
        );
        let slice_batches = encode_frame_sliced(
            &frame, 0, 0, false, 4, &mut slice_encs, &mut pkt2,
        );
        let slice_total: usize = slice_batches.iter().map(|b| b.len()).sum();

        // Slice FEC has more packets due to per-slice parity minimums (each slice
        // gets at least 1 parity shard, vs bulk which may share fewer parity shards).
        // With 4 slices, expect up to ~40% more packets.
        let ratio = slice_total as f64 / bulk_packets.len() as f64;
        assert!(ratio > 0.8 && ratio < 1.5,
            "Slice FEC packet count ({}) should be within expected range of bulk ({}), ratio={:.2}",
            slice_total, bulk_packets.len(), ratio);
    }

    #[test]
    fn test_rtp_slice_flags_encode() {
        // Verify fvp_flags::encode is now used in the bulk path too
        let frame = vec![0xFF; 100];
        let mut pkt = make_packetizer();
        let packets = encode_frame_to_packets(&frame, 0, 0, true, 0.2, &mut pkt);
        let flags = u16::from_le_bytes([packets[0].data[20], packets[0].data[21]]);
        // Should use fvp_flags::encode_simple(true) = keyframe bit set, rest zero
        assert!(fvp_common::protocol::fvp_flags::is_keyframe(flags));
        assert_eq!(fvp_common::protocol::fvp_flags::slice_index(flags), 0);
        assert_eq!(fvp_common::protocol::fvp_flags::slice_count(flags), 0);
    }

    // --- data_shard_count in the FVP header (protocol v4) ---

    use crate::transport::rtp::{read_fvp_header, write_fvp_header, write_rtp_header, FvpHeader};

    fn header_of(p: &RtpPacket) -> FvpHeader {
        read_fvp_header(&p.data).expect("packet must carry a full header")
    }

    /// Frame spanning exactly `data_shards` FEC shards (last one partial).
    fn frame_of(data_shards: usize, seed: u8) -> Vec<u8> {
        let len = FEC_SHARD_SIZE * (data_shards - 1) + 7;
        (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    /// The pre-v4 client guess this header field replaces.
    fn legacy_heuristic(total: u16) -> u16 {
        (total as f32 / 1.2) as u16
    }

    /// Feed `packets` minus the indices in `drop` and return every frame the
    /// reassembler produced.
    fn reassemble(r: &mut FecFrameReassembler, packets: &[RtpPacket], drop: &[usize]) -> Vec<ReassembledFrame> {
        packets
            .iter()
            .enumerate()
            .filter(|(i, _)| !drop.contains(i))
            .filter_map(|(_, p)| r.feed(&p.data))
            .collect()
    }

    fn assert_bulk_frame_eq(frame: &ReassembledFrame, original: &[u8]) {
        // Bulk frames come back padded to whole shards with zeros.
        assert_eq!(&frame.data[..original.len()], original);
        assert!(frame.data[original.len()..].iter().all(|&b| b == 0));
        assert_eq!(frame.data.len() % FEC_SHARD_SIZE, 0);
    }

    #[test]
    fn test_every_packet_carries_data_shard_count() {
        for redundancy in [0.05f32, 0.2, 0.4, 1.0] {
            let frame = frame_of(11, 1);
            let packets = encode_frame_to_packets(&frame, 3, 0, false, redundancy, &mut make_packetizer());
            let parity = ((11.0 * redundancy).ceil() as usize).max(1);
            assert_eq!(packets.len(), 11 + parity, "redundancy {redundancy}");
            for (i, p) in packets.iter().enumerate() {
                let h = header_of(p);
                assert_eq!(h.shard_index as usize, i);
                assert_eq!(h.shard_count as usize, 11 + parity);
                assert_eq!(h.data_shard_count, 11, "redundancy {redundancy}, shard {i}");
                assert!(h.is_valid());
            }
        }
    }

    #[test]
    fn test_legacy_total_div_1_2_heuristic_is_wrong_under_adaptive_fec() {
        // REGRESSION: the client used to guess data = total / 1.2, which is
        // only right near 20% redundancy. Adaptive FEC ranges 5%..40% (and
        // config allows up to 100%):
        //   - 5%: the guess is too LOW → RS runs on the wrong code and
        //     "recovers" garbage;
        //   - 40% / 100%: the guess is too HIGH → the receiver waits for more
        //     shards than needed and drops frames FEC could have recovered.
        // Each case loses exactly `parity` data shards — the most RS can
        // recover with the true split, which the header now carries.
        let original = frame_of(10, 9);
        for (redundancy, total) in [(0.05f32, 11u16), (0.4, 14), (1.0, 20)] {
            let packets = encode_frame_to_packets(&original, 0, 0, false, redundancy, &mut make_packetizer());
            let h = header_of(&packets[0]);
            assert_eq!(h.shard_count, total);
            assert_eq!(h.data_shard_count, 10);
            let guess = legacy_heuristic(total);
            assert_ne!(guess, 10, "heuristic happened to be right for {redundancy}");

            let parity = total as usize - 10;
            let lost: Vec<usize> = (0..parity).collect();
            let refs: Vec<&[u8]> = packets[parity..].iter().map(|p| p.data.as_slice()).collect();
            let guessed = decode_packets_to_frame(&refs, guess as usize, total as usize, original.len());
            assert_ne!(guessed.ok().as_deref(), Some(original.as_slice()),
                "guessed split {guess}/{total} must not recover the frame");

            let frames = reassemble(&mut FecFrameReassembler::new(), &packets, &lost);
            assert_eq!(frames.len(), 1, "header split recovers at {redundancy}");
            assert_bulk_frame_eq(&frames[0], &original);
        }
    }

    #[test]
    fn test_reassembler_recovers_max_loss_at_each_redundancy() {
        for redundancy in [0.05f32, 0.2, 0.4, 1.0] {
            let original = frame_of(12, 3);
            let packets = encode_frame_to_packets(&original, 5, 0, true, redundancy, &mut make_packetizer());
            let parity = packets.len() - 12;
            // Lose as many data shards as there are parity shards — the
            // most RS can recover.
            let lost: Vec<usize> = (0..parity).map(|k| (k * 5) % 12).collect();
            let lost: Vec<usize> = {
                let mut v = lost;
                v.sort_unstable();
                v.dedup();
                v
            };
            let frames = reassemble(&mut FecFrameReassembler::new(), &packets, &lost);
            assert_eq!(frames.len(), 1, "redundancy {redundancy}, lost {lost:?}");
            assert_eq!(frames[0].frame_index, 5);
            assert!(frames[0].is_keyframe);
            assert_bulk_frame_eq(&frames[0], &original);
        }
    }

    #[test]
    fn test_reassembler_follows_redundancy_changes_between_frames() {
        // One encoder + one reassembler across frames, redundancy changing
        // per frame as AdaptiveFecController would drive it.
        let mut enc = FecEncoder::new(0.05);
        let mut pkt = make_packetizer();
        let mut r = FecFrameReassembler::new();
        for (idx, redundancy) in [0.05f32, 0.4, 0.15, 1.0, 0.25].into_iter().enumerate() {
            enc.set_redundancy(redundancy);
            let original = frame_of(10, idx as u8);
            let packets = encode_frame_to_packets_with_fec(
                &original, idx as u32, 0, false, &mut enc, &mut pkt,
            );
            let frames = reassemble(&mut r, &packets, &[1]); // lose data shard 1
            assert_eq!(frames.len(), 1, "frame {idx} (redundancy {redundancy})");
            assert_eq!(frames[0].frame_index, idx as u32);
            assert_bulk_frame_eq(&frames[0], &original);
            pkt.recycle(packets);
        }
    }

    #[test]
    fn test_reassembler_emits_each_frame_once() {
        // Late parity packets after the data shards completed the frame must
        // not produce a second copy.
        let original = frame_of(4, 2);
        let packets = encode_frame_to_packets(&original, 1, 0, false, 0.5, &mut make_packetizer());
        let frames = reassemble(&mut FecFrameReassembler::new(), &packets, &[]);
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn test_reassembler_unrecoverable_frame_then_next_frame_ok() {
        let mut r = FecFrameReassembler::new();
        let first = frame_of(6, 4);
        let packets = encode_frame_to_packets(&first, 0, 0, false, 0.2, &mut make_packetizer());
        let parity = packets.len() - 6;
        let lost: Vec<usize> = (0..=parity).collect(); // one more than recoverable
        assert!(reassemble(&mut r, &packets, &lost).is_empty());

        let second = frame_of(6, 5);
        let packets = encode_frame_to_packets(&second, 1, 0, false, 0.2, &mut make_packetizer());
        let frames = reassemble(&mut r, &packets, &[]);
        assert_eq!(frames.len(), 1);
        assert_bulk_frame_eq(&frames[0], &second);
    }

    #[test]
    fn test_reassembler_sliced_frame_with_loss() {
        let original: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let mut encs: Vec<FecEncoder> = (0..4).map(|_| FecEncoder::new(0.2)).collect();
        let batches = encode_frame_sliced(&original, 8, 0, true, 4, &mut encs, &mut make_packetizer());
        let mut r = FecFrameReassembler::new();
        let mut frames = Vec::new();
        for batch in &batches {
            let data_count = header_of(&batch[0]).data_shard_count;
            assert!(data_count > 0 && (data_count as usize) < batch.len());
            // Lose the first data shard of every slice (carries the length
            // prefix, so recovery must go through RS).
            frames.extend(reassemble(&mut r, batch, &[0]));
        }
        assert_eq!(frames.len(), 1, "all slices recovered → exactly one frame");
        assert_eq!(frames[0].frame_index, 8);
        assert_eq!(frames[0].data, original, "slice prefixes stripped, no padding");
    }

    #[test]
    fn test_bulk_fallback_without_fec_reports_all_shards_as_data() {
        // RS(GF(2^8)) caps data+parity at 256; a bigger frame falls back to
        // sending data shards only. The header must then say data == total
        // (the old /1.2 guess would have expected parity that never comes).
        let original = frame_of(240, 6); // 240 data + 48 parity > 256
        let packets = encode_frame_to_packets(&original, 2, 0, true, 0.2, &mut make_packetizer());
        assert_eq!(packets.len(), 240);
        for p in &packets {
            let h = header_of(p);
            assert_eq!(h.data_shard_count, h.shard_count);
        }
        let frames = reassemble(&mut FecFrameReassembler::new(), &packets, &[]);
        assert_eq!(frames.len(), 1);
        assert_bulk_frame_eq(&frames[0], &original);
    }

    #[test]
    fn test_reassembler_drops_invalid_headers() {
        let raw = |shard_index: u16, shard_count: u16, data: u16| {
            let mut buf = Vec::new();
            write_rtp_header(&mut buf, 97, false, 0, 0, 0);
            write_fvp_header(&mut buf, 0, shard_index, shard_count, 0, data);
            buf.extend_from_slice(&[0xEE; FEC_SHARD_SIZE]);
            buf
        };
        let mut r = FecFrameReassembler::new();
        assert!(r.feed(&raw(0, 1, 0)).is_none(), "data=0");
        assert!(r.feed(&raw(0, 2, 3)).is_none(), "data>total");
        assert!(r.feed(&raw(2, 2, 1)).is_none(), "index>=total");
        assert!(r.feed(&raw(0, (MAX_FRAME_SHARDS + 1) as u16, 1)).is_none(), "total>MAX");
        assert!(r.feed(&[0u8; PACKET_HEADER_LEN - 1]).is_none(), "short packet");
        assert!(r.feed(&raw(0, 1, 1)[..PACKET_HEADER_LEN]).is_none(), "empty payload");

        // None of the garbage above poisons a following valid frame.
        let original = frame_of(3, 7);
        let packets = encode_frame_to_packets(&original, 0, 0, false, 0.4, &mut make_packetizer());
        let frames = reassemble(&mut r, &packets, &[2]);
        assert_eq!(frames.len(), 1);
        assert_bulk_frame_eq(&frames[0], &original);
    }

    #[test]
    fn test_reassembler_ignores_shards_with_conflicting_counts() {
        // A packet claiming a different data/total split for a frame already
        // in progress (forged or corrupt) is ignored rather than mixed in.
        let original = frame_of(4, 8);
        let packets = encode_frame_to_packets(&original, 0, 0, false, 0.5, &mut make_packetizer());
        let mut r = FecFrameReassembler::new();
        assert!(r.feed(&packets[0].data).is_none());
        let mut forged = packets[1].data.clone();
        forged[22..24].copy_from_slice(&5u16.to_le_bytes()); // data 4 → 5
        assert!(r.feed(&forged).is_none());
        let frames: Vec<_> = packets[1..].iter().filter_map(|p| r.feed(&p.data)).collect();
        assert_eq!(frames.len(), 1);
        assert_bulk_frame_eq(&frames[0], &original);
    }
}
