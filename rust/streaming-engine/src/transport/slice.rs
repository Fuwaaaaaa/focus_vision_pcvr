/// Splits a NAL unit into N slices at byte boundaries.
/// Remainder bytes go to the first slice (e.g., 101 bytes / 4 = 26+25+25+25).
pub struct SliceSplitter;

impl SliceSplitter {
    /// Split `data` into `count` slices. Returns Vec of byte slices.
    /// Empty slices are represented as empty &[u8].
    pub fn split(data: &[u8], count: u8) -> Vec<&[u8]> {
        let count = count as usize;
        if count == 0 || data.is_empty() {
            return vec![data];
        }

        let base_size = data.len() / count;
        let remainder = data.len() % count;
        let mut slices = Vec::with_capacity(count);
        let mut offset = 0;

        for i in 0..count {
            let size = if i < remainder { base_size + 1 } else { base_size };
            if offset + size <= data.len() {
                slices.push(&data[offset..offset + size]);
            } else if offset < data.len() {
                slices.push(&data[offset..]);
            } else {
                slices.push(&data[0..0]); // empty slice
            }
            offset += size;
        }

        slices
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slice_splitter_4_equal() {
        let data = vec![0u8; 100];
        let slices = SliceSplitter::split(&data, 4);
        assert_eq!(slices.len(), 4);
        for s in &slices {
            assert_eq!(s.len(), 25);
        }
    }

    #[test]
    fn test_slice_splitter_uneven() {
        let data = vec![0u8; 101];
        let slices = SliceSplitter::split(&data, 4);
        assert_eq!(slices.len(), 4);
        // Remainder 1 goes to first slice
        assert_eq!(slices[0].len(), 26);
        assert_eq!(slices[1].len(), 25);
        assert_eq!(slices[2].len(), 25);
        assert_eq!(slices[3].len(), 25);
        // Total preserved
        let total: usize = slices.iter().map(|s| s.len()).sum();
        assert_eq!(total, 101);
    }

    #[test]
    fn test_slice_splitter_small_frame() {
        let data = vec![1, 2, 3];
        let slices = SliceSplitter::split(&data, 4);
        assert_eq!(slices.len(), 4);
        assert_eq!(slices[0].len(), 1);
        assert_eq!(slices[1].len(), 1);
        assert_eq!(slices[2].len(), 1);
        assert_eq!(slices[3].len(), 0); // empty slice
    }

    #[test]
    fn test_slice_splitter_single_byte() {
        let data = vec![42];
        let slices = SliceSplitter::split(&data, 4);
        assert_eq!(slices.len(), 4);
        assert_eq!(slices[0], &[42]);
        assert_eq!(slices[1].len(), 0);
        assert_eq!(slices[2].len(), 0);
        assert_eq!(slices[3].len(), 0);
    }

    #[test]
    fn test_slice_splitter_empty() {
        let data: Vec<u8> = vec![];
        let slices = SliceSplitter::split(&data, 4);
        // Empty input returns single empty slice
        assert_eq!(slices.len(), 1);
        assert!(slices[0].is_empty());
    }

    #[test]
    fn test_slice_splitter_data_integrity() {
        let data: Vec<u8> = (0..200).map(|i| i as u8).collect();
        let slices = SliceSplitter::split(&data, 4);
        // Reassemble and verify
        let reassembled: Vec<u8> = slices.iter().flat_map(|s| s.iter().copied()).collect();
        assert_eq!(reassembled, data);
    }

    #[test]
    fn test_slice_splitter_count_2_and_8() {
        let data = vec![0u8; 100];

        let slices2 = SliceSplitter::split(&data, 2);
        assert_eq!(slices2.len(), 2);
        assert_eq!(slices2[0].len(), 50);
        assert_eq!(slices2[1].len(), 50);

        let slices8 = SliceSplitter::split(&data, 8);
        assert_eq!(slices8.len(), 8);
        // 100 / 8 = 12 remainder 4
        assert_eq!(slices8[0].len(), 13); // 12+1
        assert_eq!(slices8[4].len(), 12); // no remainder
        let total: usize = slices8.iter().map(|s| s.len()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn test_slice_splitter_count_zero() {
        let data = vec![1, 2, 3];
        let slices = SliceSplitter::split(&data, 0);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0], &data[..]);
    }
}
