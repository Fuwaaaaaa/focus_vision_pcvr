use reed_solomon_erasure::galois_8::ReedSolomon;

/// Forward Error Correction encoder using Reed-Solomon.
///
/// Caches the ReedSolomon instance across frames to avoid repeated
/// Galois Field table computation. The cache is invalidated when the
/// shard count changes (different frame sizes produce different shard counts).
pub struct FecEncoder {
    redundancy: f32,
    cached_rs: Option<ReedSolomon>,
    cached_data_count: usize,
    cached_parity_count: usize,
}

impl FecEncoder {
    pub fn new(redundancy: f32) -> Self {
        Self {
            redundancy: redundancy.clamp(0.0, 1.0),
            cached_rs: None,
            cached_data_count: 0,
            cached_parity_count: 0,
        }
    }

    /// Encode data shards and produce parity shards.
    /// Takes ownership of data shards to avoid cloning.
    /// Returns: data shards + parity shards concatenated.
    pub fn encode(&mut self, mut data_shards: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>, FecError> {
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

        // Reuse cached ReedSolomon if shard counts match
        if self.cached_rs.is_none()
            || self.cached_data_count != data_count
            || self.cached_parity_count != parity_count
        {
            self.cached_rs = Some(
                ReedSolomon::new(data_count, parity_count)
                    .map_err(|e| FecError::ReedSolomon(format!("{e}")))?,
            );
            self.cached_data_count = data_count;
            self.cached_parity_count = parity_count;
        }

        let rs = self.cached_rs.as_ref().unwrap();

        // Append empty parity shards
        for _ in 0..parity_count {
            data_shards.push(vec![0u8; shard_len]);
        }

        rs.encode(&mut data_shards)
            .map_err(|e| FecError::ReedSolomon(format!("{e}")))?;

        Ok(data_shards)
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
        shards: &mut [Option<Vec<u8>>],
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

impl FecEncoder {
    /// Update the redundancy ratio dynamically. Clamped to [0.0, 1.0].
    pub fn set_redundancy(&mut self, redundancy: f32) {
        let new = redundancy.clamp(0.0, 1.0);
        if (new - self.redundancy).abs() > f32::EPSILON {
            self.redundancy = new;
            // Invalidate cache so next encode() picks up the new parity count
            self.cached_rs = None;
        }
    }

    pub fn redundancy(&self) -> f32 { self.redundancy }
}

/// Adaptive FEC controller that adjusts redundancy based on observed packet loss.
///
/// Loss thresholds:
///   < 1% → 5% redundancy (bandwidth saving)
///   1-3% → 15%
///   3-5% → 25%
///   > 5% → 40% (maximum protection)
///
/// Change rate limited to ±5% per evaluation to avoid oscillation.
pub struct AdaptiveFecController {
    current_redundancy: f32,
    min_redundancy: f32,
    max_redundancy: f32,
    max_change_per_step: f32,
}

impl AdaptiveFecController {
    pub fn new(min_redundancy: f32, max_redundancy: f32, initial: f32) -> Self {
        let min = min_redundancy.clamp(0.0, 1.0);
        let max = max_redundancy.clamp(min, 1.0);
        debug_assert!(min <= max, "FEC min_redundancy ({min}) > max_redundancy ({max})");
        Self {
            current_redundancy: initial.clamp(min, max),
            min_redundancy: min,
            max_redundancy: max,
            max_change_per_step: 0.05,
        }
    }

    /// Evaluate loss rate and return the new redundancy ratio.
    /// Returns true if redundancy changed.
    pub fn adjust(&mut self, loss_rate: f64) -> bool {
        let loss = if loss_rate.is_nan() || loss_rate.is_infinite() {
            log::warn!("AdaptiveFEC: invalid loss_rate ({loss_rate}), treating as 0");
            0.0
        } else {
            loss_rate.clamp(0.0, 1.0)
        };

        let target: f32 = if loss < 0.01 {
            0.05
        } else if loss < 0.03 {
            0.15
        } else if loss < 0.05 {
            0.25
        } else {
            0.40
        };

        let target = target.clamp(self.min_redundancy, self.max_redundancy);
        let diff = target - self.current_redundancy;
        let clamped_diff = diff.clamp(-self.max_change_per_step, self.max_change_per_step);
        let new_redundancy = (self.current_redundancy + clamped_diff)
            .clamp(self.min_redundancy, self.max_redundancy);

        if (new_redundancy - self.current_redundancy).abs() > f32::EPSILON {
            log::info!("AdaptiveFEC: {:.0}% → {:.0}% (loss={:.1}%)",
                self.current_redundancy * 100.0, new_redundancy * 100.0, loss * 100.0);
            self.current_redundancy = new_redundancy;
            true
        } else {
            false
        }
    }

    pub fn current_redundancy(&self) -> f32 { self.current_redundancy }
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
        let mut encoder = FecEncoder::new(0.5); // 50% redundancy
        let data: Vec<Vec<u8>> = (0..4).map(|i| vec![i; 100]).collect();
        let expected = data.clone();
        let encoded = encoder.encode(data).unwrap();
        assert_eq!(encoded.len(), 6); // 4 data + 2 parity

        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        let decoded = FecDecoder::decode(&mut shards, 4).unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_fec_recover_from_loss() {
        let mut encoder = FecEncoder::new(0.5);
        let data: Vec<Vec<u8>> = (0..4).map(|i| vec![i; 100]).collect();
        let expected = data.clone();
        let encoded = encoder.encode(data).unwrap();

        // Lose 2 shards (within parity capacity)
        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        shards[1] = None; // lose data shard 1
        shards[3] = None; // lose data shard 3

        let decoded = FecDecoder::decode(&mut shards, 4).unwrap();
        assert_eq!(decoded, expected);
    }

    #[test]
    fn test_fec_too_much_loss() {
        let mut encoder = FecEncoder::new(0.25); // 1 parity shard for 4 data
        let data: Vec<Vec<u8>> = (0..4).map(|i| vec![i; 100]).collect();
        let encoded = encoder.encode(data).unwrap();
        assert_eq!(encoded.len(), 5); // 4 data + 1 parity

        // Lose 2 shards — exceeds parity capacity
        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        shards[0] = None;
        shards[2] = None;

        let result = FecDecoder::decode(&mut shards, 4);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_redundancy() {
        let mut enc = FecEncoder::new(0.2);
        assert!((enc.redundancy() - 0.2).abs() < 0.01);
        enc.set_redundancy(0.4);
        assert!((enc.redundancy() - 0.4).abs() < 0.01);
    }

    #[test]
    fn test_set_redundancy_clamp() {
        let mut enc = FecEncoder::new(0.2);
        enc.set_redundancy(2.0); // > 1.0
        assert!((enc.redundancy() - 1.0).abs() < 0.01);
        enc.set_redundancy(-0.5); // < 0.0
        assert!(enc.redundancy().abs() < 0.01);
    }

    #[test]
    fn test_adaptive_fec_low_loss() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        // Start at 20%, low loss → target 5%, but max step is 5%
        ctrl.adjust(0.005); // < 1%
        assert!((ctrl.current_redundancy() - 0.15).abs() < 0.01); // 20% - 5% = 15%
    }

    #[test]
    fn test_adaptive_fec_high_loss() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        // Start at 20%, high loss → target 40%, max step 5%
        ctrl.adjust(0.10); // > 5%
        assert!((ctrl.current_redundancy() - 0.25).abs() < 0.01); // 20% + 5% = 25%
    }

    #[test]
    fn test_adaptive_fec_moderate_loss() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        ctrl.adjust(0.02); // 2% → target 15%, diff = -5%
        assert!((ctrl.current_redundancy() - 0.15).abs() < 0.01);
    }

    #[test]
    fn test_adaptive_fec_step_limit() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        // Multiple steps to reach target
        ctrl.adjust(0.005); // 20→15
        ctrl.adjust(0.005); // 15→10
        ctrl.adjust(0.005); // 10→5
        assert!((ctrl.current_redundancy() - 0.05).abs() < 0.01);
        // Already at min, no further change
        let changed = ctrl.adjust(0.005);
        assert!(!changed);
    }

    #[test]
    fn test_adaptive_fec_nan_loss() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        ctrl.adjust(f64::NAN); // should treat as 0
        assert!((ctrl.current_redundancy() - 0.15).abs() < 0.01); // target 5%, step -5%
    }

    #[test]
    fn test_adaptive_fec_boundary_1_percent() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        // Exactly 1% → falls into 1-3% bracket (target 15%)
        ctrl.adjust(0.01);
        assert!((ctrl.current_redundancy() - 0.15).abs() < 0.01);
    }

    #[test]
    fn test_adaptive_fec_boundary_3_percent() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        // Exactly 3% → falls into 3-5% bracket (target 25%)
        ctrl.adjust(0.03);
        assert!((ctrl.current_redundancy() - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_adaptive_fec_boundary_5_percent() {
        let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        // Exactly 5% → loss < 0.05 is false, so falls into >=5% bracket (target 40%).
        // From 20%, step-limited to +5% = 25%.
        ctrl.adjust(0.05);
        assert!((ctrl.current_redundancy() - 0.25).abs() < 0.01);
    }

    #[test]
    fn test_adaptive_fec_initial_clamped_to_range() {
        // Initial value below min should be clamped to min
        let ctrl = AdaptiveFecController::new(0.30, 0.40, 0.20);
        assert!((ctrl.current_redundancy() - 0.30).abs() < 0.01);

        // Initial value above max should be clamped to max
        let ctrl = AdaptiveFecController::new(0.05, 0.25, 0.40);
        assert!((ctrl.current_redundancy() - 0.25).abs() < 0.01);

        // Initial value within range stays as-is
        let ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
        assert!((ctrl.current_redundancy() - 0.20).abs() < 0.01);
    }

    #[test]
    fn test_fec_20_percent_default() {
        let mut encoder = FecEncoder::new(0.2);
        let data: Vec<Vec<u8>> = (0..10).map(|i| vec![i; 200]).collect();
        let expected = data.clone();
        let encoded = encoder.encode(data).unwrap();
        assert_eq!(encoded.len(), 12); // 10 data + 2 parity

        // Lose 2 shards (exactly at capacity)
        let mut shards: Vec<Option<Vec<u8>>> = encoded.into_iter().map(Some).collect();
        shards[3] = None;
        shards[7] = None;

        let decoded = FecDecoder::decode(&mut shards, 10).unwrap();
        assert_eq!(decoded, expected);
    }
}
