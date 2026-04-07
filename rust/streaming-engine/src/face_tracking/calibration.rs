use super::profiles::{FtProfile, TOTAL_BLENDSHAPES};

/// Number of frames to collect per calibration step.
const FRAMES_PER_STEP: usize = 90; // ~1 second at 90fps

/// Calibration steps: each prompts the user to make a specific expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationStep {
    /// Step 1: Relax face completely (capture baseline minimums)
    Relax,
    /// Step 2: Open mouth wide, raise eyebrows (capture maximums)
    ExaggerateAll,
    /// Done: compute weights from collected min/max
    Done,
}

impl CalibrationStep {
    pub fn index(self) -> u8 {
        match self {
            Self::Relax => 0,
            Self::ExaggerateAll => 1,
            Self::Done => 2,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Relax => Self::ExaggerateAll,
            Self::ExaggerateAll => Self::Done,
            Self::Done => Self::Done,
        }
    }
}

/// Tracks min/max blendshape values during calibration.
pub struct CalibrationState {
    step: CalibrationStep,
    frames_collected: usize,
    min_values: [f32; TOTAL_BLENDSHAPES],
    max_values: [f32; TOTAL_BLENDSHAPES],
    profile_name: String,
}

impl CalibrationState {
    pub fn new(profile_name: &str) -> Self {
        Self {
            step: CalibrationStep::Relax,
            frames_collected: 0,
            min_values: [f32::MAX; TOTAL_BLENDSHAPES],
            max_values: [f32::MIN; TOTAL_BLENDSHAPES],
            profile_name: profile_name.to_string(),
        }
    }

    pub fn current_step(&self) -> CalibrationStep {
        self.step
    }

    pub fn frames_collected(&self) -> usize {
        self.frames_collected
    }

    pub fn frames_needed(&self) -> usize {
        FRAMES_PER_STEP
    }

    pub fn is_done(&self) -> bool {
        self.step == CalibrationStep::Done
    }

    /// Feed one frame of blendshape data. Returns true if the current step is complete.
    pub fn update(&mut self, lip: &[f32; 37], eye: &[f32; 14]) -> bool {
        if self.is_done() {
            return true;
        }

        // Combine lip + eye into one array
        for (i, &v) in lip.iter().chain(eye.iter()).enumerate() {
            if v < self.min_values[i] {
                self.min_values[i] = v;
            }
            if v > self.max_values[i] {
                self.max_values[i] = v;
            }
        }

        self.frames_collected += 1;

        if self.frames_collected >= FRAMES_PER_STEP {
            self.frames_collected = 0;
            self.step = self.step.next();
            true // Step complete
        } else {
            false
        }
    }

    /// Compute the profile from collected calibration data.
    /// Weight = 1.0 / (max - min) for each blendshape.
    /// If max == min (no range), weight defaults to 1.0.
    pub fn compute_profile(&self) -> FtProfile {
        let mut weights = vec![1.0f32; TOTAL_BLENDSHAPES];

        for (i, weight) in weights.iter_mut().enumerate() {
            let range = self.max_values[i] - self.min_values[i];
            if range > 0.01 {
                *weight = 1.0 / range;
            }
        }

        FtProfile {
            name: self.profile_name.clone(),
            weights,
            smoothing_override: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibration_steps() {
        assert_eq!(CalibrationStep::Relax.next(), CalibrationStep::ExaggerateAll);
        assert_eq!(CalibrationStep::ExaggerateAll.next(), CalibrationStep::Done);
        assert_eq!(CalibrationStep::Done.next(), CalibrationStep::Done);
    }

    #[test]
    fn test_calibration_state_new() {
        let state = CalibrationState::new("test");
        assert_eq!(state.current_step(), CalibrationStep::Relax);
        assert_eq!(state.frames_collected(), 0);
        assert!(!state.is_done());
    }

    #[test]
    fn test_calibration_collects_frames() {
        let mut state = CalibrationState::new("test");
        let lip = [0.5f32; 37];
        let eye = [0.3f32; 14];

        // Feed frames until step completes
        for _ in 0..FRAMES_PER_STEP - 1 {
            assert!(!state.update(&lip, &eye));
        }
        // Last frame of step should return true
        assert!(state.update(&lip, &eye));
        assert_eq!(state.current_step(), CalibrationStep::ExaggerateAll);
    }

    #[test]
    fn test_calibration_full_flow() {
        let mut state = CalibrationState::new("avatar1");

        // Step 1: Relax (low values)
        let lip_low = [0.1f32; 37];
        let eye_low = [0.05f32; 14];
        for _ in 0..FRAMES_PER_STEP {
            state.update(&lip_low, &eye_low);
        }
        assert_eq!(state.current_step(), CalibrationStep::ExaggerateAll);

        // Step 2: Exaggerate (high values)
        let lip_high = [0.9f32; 37];
        let eye_high = [0.8f32; 14];
        for _ in 0..FRAMES_PER_STEP {
            state.update(&lip_high, &eye_high);
        }
        assert!(state.is_done());

        // Compute profile
        let profile = state.compute_profile();
        assert_eq!(profile.name, "avatar1");
        assert_eq!(profile.weights.len(), TOTAL_BLENDSHAPES);

        // Weight for lip: 1.0 / (0.9 - 0.1) = 1.25
        assert!((profile.weights[0] - 1.25).abs() < 0.01);
        // Weight for eye: 1.0 / (0.8 - 0.05) = 1.333
        assert!((profile.weights[37] - 1.333).abs() < 0.01);
    }

    #[test]
    fn test_calibration_constant_values_no_division_by_zero() {
        let mut state = CalibrationState::new("constant");
        let lip = [0.5f32; 37];
        let eye = [0.5f32; 14];

        // All frames have identical values → range = 0
        for _ in 0..FRAMES_PER_STEP * 2 {
            state.update(&lip, &eye);
        }

        let profile = state.compute_profile();
        // Weight should be 1.0 (default) when range < 0.01
        assert!(profile.weights.iter().all(|&w| (w - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    fn test_calibration_step_indices() {
        assert_eq!(CalibrationStep::Relax.index(), 0);
        assert_eq!(CalibrationStep::ExaggerateAll.index(), 1);
        assert_eq!(CalibrationStep::Done.index(), 2);
    }
}
