use fvp_common::{MTU_SIZE, RTP_PT_H265};

/// Maximum payload per RTP packet (MTU minus RTP header 12B minus FVP header 10B)
const MAX_PAYLOAD: usize = MTU_SIZE - 12 - 10;

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

            let mut buf = self.take_buf(12 + 10 + chunk.len());

            // RTP header (12 bytes)
            write_rtp_header(&mut buf, RTP_PT_H265, is_last, seq, timestamp_90khz, self.ssrc);

            // FVP header (10 bytes) — shard fields are u16 to support large keyframes
            buf.extend_from_slice(&frame_index.to_le_bytes());
            buf.extend_from_slice(&(i as u16).to_le_bytes());            // shard_index
            buf.extend_from_slice(&(total_chunks as u16).to_le_bytes()); // shard_count
            let flags: u16 = if is_keyframe { 1 } else { 0 };
            buf.extend_from_slice(&flags.to_le_bytes());

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
        if packet.len() < 22 {
            return None; // Too small (12 RTP + 10 FVP minimum)
        }

        // Parse RTP header
        let _mpt = packet[1];
        let marker = (_mpt & 0x80) != 0;

        // Parse FVP header (bytes 12..22) — shard fields are u16
        let frame_index = u32::from_le_bytes([packet[12], packet[13], packet[14], packet[15]]);
        let shard_index = u16::from_le_bytes([packet[16], packet[17]]) as usize;
        let shard_count = u16::from_le_bytes([packet[18], packet[19]]) as usize;
        let flags = u16::from_le_bytes([packet[20], packet[21]]);
        let is_keyframe = (flags & 1) != 0;

        // Payload starts at byte 22
        let payload = &packet[22..];

        // Sanity check: reject absurd shard counts to prevent memory exhaustion
        const MAX_SHARDS: usize = 4096; // ~5MB at 1200B/shard — far beyond any real frame
        if shard_count == 0 || shard_count > MAX_SHARDS || shard_index >= shard_count {
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
    fn test_packetize_small_frame() {
        let mut pkt = RtpPacketizer::new(0x12345678);
        let frame = vec![0xAA; 100]; // Small frame, fits in 1 packet
        let packets = pkt.packetize(&frame, 0, 0, true);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].data.len(), 12 + 10 + 100);
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
