use fvp_common::{MAX_FRAME_SHARDS, MTU_SIZE, PACKET_HEADER_LEN, RTP_HEADER_LEN, RTP_PT_H265};

/// Maximum payload per RTP packet (MTU minus RTP header 12B minus FVP header 12B)
const MAX_PAYLOAD: usize = MTU_SIZE - PACKET_HEADER_LEN;

/// Append a 12-byte RTP header to `buf`.
/// Format: V=2, P=0, X=0, CC=0 | M,PT | seq(BE) | timestamp(BE) | SSRC(BE).
pub fn write_rtp_header(
    buf: &mut Vec<u8>,
    payload_type: u8,
    marker: bool,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
) {
    buf.push(0x80); // V=2, P=0, X=0, CC=0
    let mpt = if marker { 0x80 | (payload_type & 0x7F) } else { payload_type & 0x7F };
    buf.push(mpt);
    buf.extend_from_slice(&sequence.to_be_bytes());
    buf.extend_from_slice(&timestamp.to_be_bytes());
    buf.extend_from_slice(&ssrc.to_be_bytes());
}

/// Append the 12-byte FVP header that follows the RTP header.
/// Layout: frame_index u32 LE | shard_index u16 LE | shard_count u16 LE |
/// flags u16 LE | data_shard_count u16 LE.
///
/// `data_shard_count` (protocol v4) is appended after `flags` so the older
/// fields keep their offsets; the payload starts at `PACKET_HEADER_LEN` (24).
pub fn write_fvp_header(
    buf: &mut Vec<u8>,
    frame_index: u32,
    shard_index: u16,
    shard_count: u16,
    flags: u16,
    data_shard_count: u16,
) {
    buf.extend_from_slice(&frame_index.to_le_bytes());
    buf.extend_from_slice(&shard_index.to_le_bytes());
    buf.extend_from_slice(&shard_count.to_le_bytes());
    buf.extend_from_slice(&flags.to_le_bytes());
    buf.extend_from_slice(&data_shard_count.to_le_bytes());
}

/// Parsed FVP header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FvpHeader {
    pub frame_index: u32,
    pub shard_index: u16,
    /// Total shards (data + parity) in the frame, or in the slice when
    /// `fvp_flags::slice_count(flags) > 0`.
    pub shard_count: u16,
    pub flags: u16,
    /// How many of `shard_count` are data shards (the rest are RS parity).
    pub data_shard_count: u16,
}

impl FvpHeader {
    pub fn is_keyframe(self) -> bool { (self.flags & 1) != 0 }

    /// Structural validity of the shard fields. Receivers must drop packets
    /// that fail this: the counts size receive buffers and index them, so a
    /// forged header (data > total, index >= total, huge total) would
    /// otherwise drive out-of-range access or unbounded allocation.
    pub fn is_valid(self) -> bool {
        let total = self.shard_count as usize;
        total > 0
            && total <= MAX_FRAME_SHARDS
            && self.shard_index < self.shard_count
            && self.data_shard_count > 0
            && self.data_shard_count <= self.shard_count
    }
}

/// Parse the 12-byte FVP header from a packet.
/// Returns None if the packet is shorter than `PACKET_HEADER_LEN` (12 RTP +
/// 12 FVP). Field values are returned as-is — call `FvpHeader::is_valid`
/// before trusting the shard counts.
pub fn read_fvp_header(packet: &[u8]) -> Option<FvpHeader> {
    if packet.len() < PACKET_HEADER_LEN {
        return None;
    }
    let f = &packet[RTP_HEADER_LEN..PACKET_HEADER_LEN];
    Some(FvpHeader {
        frame_index: u32::from_le_bytes([f[0], f[1], f[2], f[3]]),
        shard_index: u16::from_le_bytes([f[4], f[5]]),
        shard_count: u16::from_le_bytes([f[6], f[7]]),
        flags: u16::from_le_bytes([f[8], f[9]]),
        data_shard_count: u16::from_le_bytes([f[10], f[11]]),
    })
}

/// A single RTP packet ready for transmission.
#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub data: Vec<u8>,
}

/// Packetizes encoded NAL units into RTP packets with FVP headers.
/// Maintains a pool of reusable byte buffers to avoid per-frame allocation.
pub struct RtpPacketizer {
    ssrc: u32,
    sequence: u16,
    /// Recycled packet buffers from previous frames.
    /// After the first frame, subsequent frames reuse these without allocating.
    buf_pool: Vec<Vec<u8>>,
}

impl RtpPacketizer {
    pub fn new(ssrc: u32) -> Self {
        Self { ssrc, sequence: 0, buf_pool: Vec::new() }
    }

    /// Take a buffer from the pool (reusing capacity) or create a new one.
    pub(crate) fn take_buf(&mut self, needed: usize) -> Vec<u8> {
        match self.buf_pool.pop() {
            Some(mut buf) => {
                buf.clear();
                buf.reserve(needed.saturating_sub(buf.capacity()));
                buf
            }
            None => Vec::with_capacity(needed),
        }
    }

    /// Return used packet buffers to the pool for reuse on the next frame.
    pub fn recycle(&mut self, packets: Vec<RtpPacket>) {
        for pkt in packets {
            self.buf_pool.push(pkt.data);
        }
    }

    /// Packetize a single encoded frame into multiple RTP packets.
    /// Returns a list of packets. The last packet has the marker bit set.
    pub fn packetize(
        &mut self,
        frame_data: &[u8],
        frame_index: u32,
        timestamp_90khz: u32,
        is_keyframe: bool,
    ) -> Vec<RtpPacket> {
        if frame_data.is_empty() {
            return vec![];
        }

        // Split frame into chunks that fit in one RTP packet
        let chunks: Vec<&[u8]> = frame_data.chunks(MAX_PAYLOAD).collect();
        let total_chunks = chunks.len();
        if total_chunks > u16::MAX as usize {
            log::error!("Frame too large: {} shards exceeds u16 max. Dropping frame.", total_chunks);
            return vec![];
        }
        let mut packets = Vec::with_capacity(total_chunks);

        for (i, chunk) in chunks.iter().enumerate() {
            let is_last = i == total_chunks - 1;
            let seq = self.next_sequence();

            let mut buf = self.take_buf(PACKET_HEADER_LEN + chunk.len());

            // RTP header (12 bytes)
            write_rtp_header(&mut buf, RTP_PT_H265, is_last, seq, timestamp_90khz, self.ssrc);

            // FVP header (12 bytes) — shard fields are u16 to support large keyframes.
            // No FEC on this path, so every shard is a data shard.
            let flags: u16 = if is_keyframe { 1 } else { 0 };
            write_fvp_header(
                &mut buf, frame_index, i as u16, total_chunks as u16, flags, total_chunks as u16,
            );

            // Payload
            buf.extend_from_slice(chunk);

            packets.push(RtpPacket { data: buf });
        }

        packets
    }

    pub fn next_sequence(&mut self) -> u16 {
        let seq = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        seq
    }
}

/// Reassembles RTP packets back into complete frames.
pub struct RtpDepacketizer {
    /// Pending shards for the current frame: (shard_index -> payload)
    current_frame_index: Option<u32>,
    shards: Vec<Option<Vec<u8>>>,
    expected_count: usize,
}

impl Default for RtpDepacketizer {
    fn default() -> Self {
        Self::new()
    }
}

impl RtpDepacketizer {
    pub fn new() -> Self {
        Self {
            current_frame_index: None,
            shards: Vec::new(),
            expected_count: 0,
        }
    }

    /// Feed an RTP packet. Returns Some(frame_data) when a complete frame is assembled.
    pub fn feed(&mut self, packet: &[u8]) -> Option<ReassembledFrame> {
        // Parse RTP marker bit from byte 1 (length is implicitly validated by read_fvp_header).
        let hdr = read_fvp_header(packet)?;
        let marker = (packet[1] & 0x80) != 0;

        let frame_index = hdr.frame_index;
        let shard_index = hdr.shard_index as usize;
        let shard_count = hdr.shard_count as usize;
        let is_keyframe = hdr.is_keyframe();

        let payload = &packet[PACKET_HEADER_LEN..];

        // Sanity check: reject absurd / inconsistent shard counts to prevent
        // memory exhaustion and out-of-range indexing.
        if !hdr.is_valid() {
            return None;
        }

        // New frame?
        if self.current_frame_index != Some(frame_index) {
            // Start collecting new frame
            self.current_frame_index = Some(frame_index);
            self.shards = vec![None; shard_count];
            self.expected_count = shard_count;
        }

        // Store shard
        if shard_index < self.shards.len() {
            self.shards[shard_index] = Some(payload.to_vec());
        }

        // Check if frame is complete
        if marker || self.shards.iter().all(|s| s.is_some()) {
            // Reassemble
            let received = self.shards.iter().filter(|s| s.is_some()).count();
            if received == self.expected_count {
                let mut frame_data = Vec::new();
                for data in self.shards.iter().flatten() {
                    frame_data.extend_from_slice(data);
                }
                self.current_frame_index = None;
                return Some(ReassembledFrame {
                    frame_index,
                    is_keyframe,
                    data: frame_data,
                });
            }
        }

        None
    }
}

#[derive(Debug)]
pub struct ReassembledFrame {
    pub frame_index: u32,
    pub is_keyframe: bool,
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_rtp_header_layout() {
        let mut buf = Vec::new();
        write_rtp_header(&mut buf, 96, true, 0x1234, 0xAABBCCDD, 0x11223344);
        assert_eq!(buf.len(), 12);
        assert_eq!(buf[0], 0x80); // V=2
        assert_eq!(buf[1], 0x80 | 96); // marker + PT
        assert_eq!(&buf[2..4], &[0x12, 0x34]); // seq BE
        assert_eq!(&buf[4..8], &[0xAA, 0xBB, 0xCC, 0xDD]); // timestamp BE
        assert_eq!(&buf[8..12], &[0x11, 0x22, 0x33, 0x44]); // SSRC BE
    }

    #[test]
    fn test_write_rtp_header_no_marker() {
        let mut buf = Vec::new();
        write_rtp_header(&mut buf, 96, false, 0, 0, 0);
        assert_eq!(buf[1], 96); // marker bit clear
    }

    #[test]
    fn test_write_rtp_header_masks_pt_overflow() {
        let mut buf = Vec::new();
        write_rtp_header(&mut buf, 0xFF, false, 0, 0, 0);
        // PT is 7 bits; top bit must be zeroed (would be misinterpreted as marker)
        assert_eq!(buf[1] & 0x80, 0);
        assert_eq!(buf[1] & 0x7F, 0x7F);
    }

    #[test]
    fn test_read_fvp_header_roundtrip() {
        let mut buf = Vec::new();
        write_rtp_header(&mut buf, 96, true, 0, 0, 0);
        write_fvp_header(&mut buf, 0xDEAD_BEEF, 7, 42, 0b11, 35);
        assert_eq!(buf.len(), PACKET_HEADER_LEN);
        let hdr = read_fvp_header(&buf).unwrap();
        assert_eq!(hdr.frame_index, 0xDEAD_BEEF);
        assert_eq!(hdr.shard_index, 7);
        assert_eq!(hdr.shard_count, 42);
        assert_eq!(hdr.flags, 0b11);
        assert_eq!(hdr.data_shard_count, 35);
        assert!(hdr.is_keyframe());
        assert!(hdr.is_valid());
    }

    #[test]
    fn test_fvp_header_byte_layout() {
        // Wire contract shared with the C++ client (client_protocol.h
        // parseFvpHeader): v3 fields keep their offsets, data_shard_count is
        // appended at [22..24], payload starts at 24.
        let mut buf = Vec::new();
        write_rtp_header(&mut buf, 97, false, 0, 0, 0);
        write_fvp_header(&mut buf, 0x0403_0201, 0x0605, 0x0807, 0x0A09, 0x0C0B);
        assert_eq!(&buf[12..16], &[0x01, 0x02, 0x03, 0x04]); // frame_index LE
        assert_eq!(&buf[16..18], &[0x05, 0x06]); // shard_index LE
        assert_eq!(&buf[18..20], &[0x07, 0x08]); // shard_count LE
        assert_eq!(&buf[20..22], &[0x09, 0x0A]); // flags LE
        assert_eq!(&buf[22..24], &[0x0B, 0x0C]); // data_shard_count LE
    }

    #[test]
    fn test_read_fvp_header_too_short() {
        assert!(read_fvp_header(&[]).is_none());
        // A v3-sized (22-byte) header no longer carries enough bytes.
        assert!(read_fvp_header(&[0u8; 22]).is_none());
        assert!(read_fvp_header(&[0u8; PACKET_HEADER_LEN - 1]).is_none());
        assert!(read_fvp_header(&[0u8; PACKET_HEADER_LEN]).is_some());
    }

    fn header(shard_index: u16, shard_count: u16, data_shard_count: u16) -> FvpHeader {
        FvpHeader { frame_index: 0, shard_index, shard_count, flags: 0, data_shard_count }
    }

    #[test]
    fn test_fvp_header_validation() {
        assert!(header(0, 1, 1).is_valid());
        assert!(header(13, 14, 10).is_valid());
        assert!(header(0, MAX_FRAME_SHARDS as u16, 1).is_valid());
        // data_shard_count must be in 1..=shard_count
        assert!(!header(0, 14, 0).is_valid(), "data=0 must be rejected");
        assert!(!header(0, 14, 15).is_valid(), "data>total must be rejected");
        // shard_index must be < shard_count, shard_count in 1..=MAX
        assert!(!header(14, 14, 10).is_valid());
        assert!(!header(0, 0, 0).is_valid());
        assert!(!header(0, MAX_FRAME_SHARDS as u16 + 1, 1).is_valid());
    }

    #[test]
    fn test_depacketizer_rejects_invalid_data_shard_count() {
        let mut buf = Vec::new();
        write_rtp_header(&mut buf, 97, true, 0, 0, 0);
        write_fvp_header(&mut buf, 0, 0, 1, 0, 0); // data=0
        buf.extend_from_slice(&[1, 2, 3]);
        assert!(RtpDepacketizer::new().feed(&buf).is_none());
    }

    #[test]
    fn test_packetize_small_frame() {
        let mut pkt = RtpPacketizer::new(0x12345678);
        let frame = vec![0xAA; 100]; // Small frame, fits in 1 packet
        let packets = pkt.packetize(&frame, 0, 0, true);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].data.len(), PACKET_HEADER_LEN + 100);
        let hdr = read_fvp_header(&packets[0].data).unwrap();
        // No FEC on this path: every shard is a data shard.
        assert_eq!(hdr.data_shard_count, hdr.shard_count);
        // Marker bit should be set on the only packet
        assert_ne!(packets[0].data[1] & 0x80, 0);
    }

    #[test]
    fn test_packetize_large_frame() {
        let mut pkt = RtpPacketizer::new(1);
        let frame = vec![0xBB; MAX_PAYLOAD * 3 + 50]; // 3 full + 1 partial
        let packets = pkt.packetize(&frame, 1, 9000, false);
        assert_eq!(packets.len(), 4);
        // Only last packet should have marker bit
        for (i, p) in packets.iter().enumerate() {
            if i == 3 {
                assert_ne!(p.data[1] & 0x80, 0, "Last packet should have marker");
            } else {
                assert_eq!(p.data[1] & 0x80, 0, "Non-last packet should not have marker");
            }
        }
    }

    #[test]
    fn test_packetize_depacketize_roundtrip() {
        let mut pktizer = RtpPacketizer::new(42);
        let original = vec![0xCC; 5000];
        let packets = pktizer.packetize(&original, 7, 12345, true);

        let mut depkt = RtpDepacketizer::new();
        let mut result = None;
        for p in &packets {
            result = depkt.feed(&p.data);
        }

        let frame = result.expect("Should have reassembled frame");
        assert_eq!(frame.frame_index, 7);
        assert!(frame.is_keyframe);
        assert_eq!(frame.data, original);
    }

    #[test]
    fn test_sequence_number_wraps() {
        let mut pkt = RtpPacketizer::new(1);
        pkt.sequence = u16::MAX;
        let packets = pkt.packetize(&[0; 10], 0, 0, false);
        assert_eq!(packets.len(), 1);
        // Next call should wrap
        let packets2 = pkt.packetize(&[0; 10], 1, 0, false);
        assert_eq!(packets2.len(), 1);
    }

    #[test]
    fn test_empty_frame() {
        let mut pkt = RtpPacketizer::new(1);
        let packets = pkt.packetize(&[], 0, 0, false);
        assert!(packets.is_empty());
    }
}
