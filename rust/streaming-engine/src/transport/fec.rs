use reed_solomon_erasure::galois_8::ReedSolomon;

/// Forward Error Correction encoder using Reed-Solomon.
pub struct FecEncoder {
    redundancy: f32,
}

impl FecEncoder {
    pub fn new(redundancy: f32) -> Self {
        Self {
            redundancy: redundancy.clamp(0.0, 1.0),
        }
    }

    /// Encode data shards and produce parity shards.
    /// Input: list of equal-length data shards.
    /// Returns: data shards + parity shards concatenated.
    pub fn encode(&self, data_shards: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, FecError> {
        if data_shards.is_empty() {
            return Err(FecError::EmptyInput);
        }

        let data_count = data_shards.len();
        let parity_count = ((data_count as f32 * self.redundancy).ceil() as usize).max(1);
        let shard_len = data_shards[0].len();

        // Ensure all shards are equal length
        if data_shards.iter().any(|s| s.len() != shard_len) {
            return Err(FecError::UnequalShards);
        }

        let rs = ReedSolomon::new(data_count, parity_count)
            .map_err(|e| FecError::ReedSolomon(format!("{e}")))?;

        // Build shard matrix: data + empty parity
        let mut shards: Vec<Vec<u8>> = data_shards.to_vec();
        for _ in 0..parity_count {
            shards.push(vec![0u8; shard_len]);
        }

        rs.encode(&mut shards)
            .map_err(|e| FecError::ReedSolomon(format!("{e}")))?;

        Ok(shards)
    }
}

/// Forward Error Correction decoder using Reed-Solomon.
pub struct FecDecoder;

impl FecDecoder {
    /// Reconstruct missing shards from available data + parity.
    /// `shards`: Vec of Option<Vec<u8>>. None = lost shard.
    /// `data_count`: number of data shards (first N in the array).
    /// Returns reconstructed data shards on success.
    pub fn decode(
        shards: &mut Vec<Option<Vec<u8>>>,
        data_count: usize,
    ) -> Result<Vec<Vec<u8>>, FecError> {
        let total = shards.len();
        if total <= data_count {
            return Err(FecError::InsufficientShards);
        }
        let parity_count = total - data_count;

        let rs = ReedSolomon::new(data_count, parity_count)
            .map_err(|e| FecError::ReedSolomon(format!("{e}")))?;

        rs.reconstruct(shards)
            .map_err(|e| FecError::ReedSolomon(format!("{e}")))?;

        // Extract data shards
        let result: Vec<Vec<u8>> = shards[..data_count]
            .iter()
            .map(|s| s.as_ref().expect("reconstructed").clone())
            .collect();

        Ok(result)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FecError {
    #[error("empty input")]
    EmptyInput,
    #[error("shards have unequal length")]
    UnequalShards,
    #[error("insufficient shards for reconstruction")]
    InsufficientShards,
    #[error("reed-solomon error: {0}")]
    ReedSolomon(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fec_encode_decode_no_loss() {
        let encoder = FecEncoder::new(0.5); // 50% redundancy
        let data: Vec<Vec<u8>> = (0..4).map(|i| vec![i; 100]).collect();
        let encoded = encoder.encode(&data).unwrap();
        assert_eq!(encoded.len(), 6); // 4 data + 2 parity

        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        let decoded = FecDecoder::decode(&mut shards, 4).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_fec_recover_from_loss() {
        let encoder = FecEncoder::new(0.5);
        let data: Vec<Vec<u8>> = (0..4).map(|i| vec![i; 100]).collect();
        let encoded = encoder.encode(&data).unwrap();

        // Lose 2 shards (within parity capacity)
        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        shards[1] = None; // lose data shard 1
        shards[3] = None; // lose data shard 3

        let decoded = FecDecoder::decode(&mut shards, 4).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_fec_too_much_loss() {
        let encoder = FecEncoder::new(0.25); // 1 parity shard for 4 data
        let data: Vec<Vec<u8>> = (0..4).map(|i| vec![i; 100]).collect();
        let encoded = encoder.encode(&data).unwrap();
        assert_eq!(encoded.len(), 5); // 4 data + 1 parity

        // Lose 2 shards — exceeds parity capacity
        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        shards[0] = None;
        shards[2] = None;

        let result = FecDecoder::decode(&mut shards, 4);
        assert!(result.is_err());
    }

    #[test]
    fn test_fec_20_percent_default() {
        let encoder = FecEncoder::new(0.2);
        let data: Vec<Vec<u8>> = (0..10).map(|i| vec![i; 200]).collect();
        let encoded = encoder.encode(&data).unwrap();
        assert_eq!(encoded.len(), 12); // 10 data + 2 parity

        // Lose 2 shards (exactly at capacity)
        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        shards[3] = None;
        shards[7] = None;

        let decoded = FecDecoder::decode(&mut shards, 10).unwrap();
        assert_eq!(decoded, data);
    }
}
