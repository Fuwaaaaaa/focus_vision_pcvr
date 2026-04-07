use fvp_common::protocol::TrackingData;

/// Detects user inactivity from head pose changes and manages sleep/wake transitions.
pub struct SleepDetector {
    enabled: bool,
    motion_threshold: f32,
    timeout_seconds: u32,
    sleep_bitrate_mbps: u32,

    prev_position: Option<[f32; 3]>,
    idle_frames: u64,
    is_sleeping: bool,
}

impl SleepDetector {
    pub fn new(enabled: bool, motion_threshold: f32, timeout_seconds: u32, sleep_bitrate_mbps: u32) -> Self {
        Self {
            enabled,
            motion_threshold,
            timeout_seconds,
            sleep_bitrate_mbps,
            prev_position: None,
            idle_frames: 0,
            is_sleeping: false,
        }
    }

    /// Feed a tracking sample. Returns state transition if any.
    /// Called at ~90Hz from the streaming loop.
    pub fn update(&mut self, tracking: &TrackingData) -> Option<SleepTransition> {
        if !self.enabled {
            return None;
        }

        let pos = tracking.position;

        let motion = match self.prev_position {
            Some(prev) => {
                let dx = pos[0] - prev[0];
                let dy = pos[1] - prev[1];
                let dz = pos[2] - prev[2];
                (dx * dx + dy * dy + dz * dz).sqrt()
            }
            None => 0.0,
        };
        self.prev_position = Some(pos);

        if motion > self.motion_threshold {
            self.idle_frames = 0;
            if self.is_sleeping {
                self.is_sleeping = false;
                return Some(SleepTransition::Wake);
            }
        } else {
            self.idle_frames += 1;
            // 90fps × timeout_seconds
            let threshold_frames = 90u64 * self.timeout_seconds as u64;
            if !self.is_sleeping && self.idle_frames >= threshold_frames {
                self.is_sleeping = true;
                return Some(SleepTransition::Sleep);
            }
        }

        None
    }

    pub fn is_sleeping(&self) -> bool {
        self.is_sleeping
    }

    pub fn sleep_bitrate_mbps(&self) -> u32 {
        self.sleep_bitrate_mbps
    }

    /// Force wake (e.g., on button press or controller input).
    pub fn force_wake(&mut self) -> Option<SleepTransition> {
        self.idle_frames = 0;
        if self.is_sleeping {
            self.is_sleeping = false;
            Some(SleepTransition::Wake)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepTransition {
    Sleep,
    Wake,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tracking(x: f32, y: f32, z: f32) -> TrackingData {
        TrackingData {
            position: [x, y, z],
            orientation: [0.0, 0.0, 0.0, 1.0],
            timestamp_ns: 0,
            gaze_x: 0.5,
            gaze_y: 0.5,
            gaze_valid: 0,
        }
    }

    #[test]
    fn test_sleep_after_timeout() {
        let mut det = SleepDetector::new(true, 0.001, 1, 8); // 1 second timeout
        let still = make_tracking(0.0, 1.5, 0.0);

        // Feed frames of no motion. Sleep triggers when idle_frames >= 90.
        // First frame sets prev_position + increments idle to 1, so sleep
        // triggers on the 90th frame (0-indexed: i=89).
        let mut slept = false;
        for i in 0..100 {
            let result = det.update(&still);
            if result == Some(SleepTransition::Sleep) {
                assert!(!slept, "Should only sleep once");
                assert!(i >= 89, "Should not sleep before ~90 frames, slept at {i}");
                slept = true;
            }
        }
        assert!(slept, "Should have entered sleep mode");
        assert!(det.is_sleeping());
    }

    #[test]
    fn test_wake_on_motion() {
        let mut det = SleepDetector::new(true, 0.001, 1, 8);
        let still = make_tracking(0.0, 1.5, 0.0);

        // Enter sleep
        for _ in 0..91 {
            det.update(&still);
        }
        assert!(det.is_sleeping());

        // Move head
        let moved = make_tracking(0.1, 1.5, 0.0);
        let result = det.update(&moved);
        assert_eq!(result, Some(SleepTransition::Wake));
        assert!(!det.is_sleeping());
    }

    #[test]
    fn test_disabled_never_sleeps() {
        let mut det = SleepDetector::new(false, 0.001, 1, 8);
        let still = make_tracking(0.0, 1.5, 0.0);

        for _ in 0..200 {
            assert_eq!(det.update(&still), None);
        }
        assert!(!det.is_sleeping());
    }

    #[test]
    fn test_force_wake() {
        let mut det = SleepDetector::new(true, 0.001, 1, 8);
        let still = make_tracking(0.0, 1.5, 0.0);

        // Enter sleep
        for _ in 0..91 {
            det.update(&still);
        }
        assert!(det.is_sleeping());

        // Force wake (e.g., button press)
        let result = det.force_wake();
        assert_eq!(result, Some(SleepTransition::Wake));
        assert!(!det.is_sleeping());
    }

    #[test]
    fn test_motion_resets_idle_counter() {
        let mut det = SleepDetector::new(true, 0.001, 1, 8);
        let still = make_tracking(0.0, 1.5, 0.0);

        // 80 frames idle (not enough for sleep at 90 threshold)
        for _ in 0..80 {
            det.update(&still);
        }

        // Move — resets counter
        let moved = make_tracking(0.05, 1.5, 0.0);
        det.update(&moved);

        // 80 more frames idle — still not enough total
        for _ in 0..80 {
            det.update(&still);
        }
        assert!(!det.is_sleeping());
    }
}
