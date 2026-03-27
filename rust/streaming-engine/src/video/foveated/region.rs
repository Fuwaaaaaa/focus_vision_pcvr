/// Foveated encoding region — defines quality zones based on gaze position.
///
/// The frame is divided into concentric regions around the gaze point:
///
/// ```text
///   ┌────────────────────────────────────┐
///   │           PERIPHERAL (low)         │
///   │    ┌────────────────────────┐      │
///   │    │     MID (medium)       │      │
///   │    │   ┌──────────────┐    │      │
///   │    │   │  FOVEA (high)│    │      │
///   │    │   │    (gaze)    │    │      │
///   │    │   └──────────────┘    │      │
///   │    └────────────────────────┘      │
///   └────────────────────────────────────┘
/// ```
///
/// When eye tracking is unavailable, the gaze defaults to frame center.

/// Gaze position in normalized coordinates (0.0-1.0).
#[derive(Debug, Clone, Copy)]
pub struct GazePoint {
    pub x: f32, // 0.0 = left, 1.0 = right
    pub y: f32, // 0.0 = top, 1.0 = bottom
}

impl Default for GazePoint {
    fn default() -> Self {
        Self { x: 0.5, y: 0.5 } // Frame center
    }
}

/// Quality region around the gaze point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FoveationLevel {
    /// Sharp center — full resolution, high QP
    Fovea,
    /// Transition zone — medium quality
    Mid,
    /// Outer edge — low quality, saves most bandwidth
    Peripheral,
}

/// Region definition with radius and quality parameters.
#[derive(Debug, Clone, Copy)]
pub struct FoveationRegion {
    pub level: FoveationLevel,
    /// Radius from gaze point, in fraction of frame width (0.0-1.0).
    pub radius: f32,
    /// QP offset from base. 0 = base quality, positive = lower quality.
    pub qp_offset: i32,
    /// Bitrate fraction allocated to this region (0.0-1.0).
    pub bitrate_fraction: f32,
}

/// Foveation configuration — three concentric regions.
#[derive(Debug, Clone)]
pub struct FoveationConfig {
    pub enabled: bool,
    pub gaze: GazePoint,
    pub fovea: FoveationRegion,
    pub mid: FoveationRegion,
    pub peripheral: FoveationRegion,
}

impl Default for FoveationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gaze: GazePoint::default(),
            fovea: FoveationRegion {
                level: FoveationLevel::Fovea,
                radius: 0.15,          // 15% of frame width
                qp_offset: 0,          // Best quality
                bitrate_fraction: 0.50, // 50% of bitrate for center
            },
            mid: FoveationRegion {
                level: FoveationLevel::Mid,
                radius: 0.35,          // 35% of frame width
                qp_offset: 5,          // Slightly lower quality
                bitrate_fraction: 0.30, // 30% of bitrate
            },
            peripheral: FoveationRegion {
                level: FoveationLevel::Peripheral,
                radius: 1.0,           // Rest of frame
                qp_offset: 15,         // Much lower quality
                bitrate_fraction: 0.20, // 20% of bitrate
            },
        }
    }
}

impl FoveationConfig {
    /// Update gaze position from eye tracker data.
    /// Coordinates are normalized: (0,0) = top-left, (1,1) = bottom-right.
    pub fn update_gaze(&mut self, x: f32, y: f32) {
        self.gaze.x = x.clamp(0.0, 1.0);
        self.gaze.y = y.clamp(0.0, 1.0);
    }

    /// Determine which region a pixel belongs to, given its normalized position.
    pub fn classify_pixel(&self, px: f32, py: f32) -> FoveationLevel {
        let dx = px - self.gaze.x;
        let dy = py - self.gaze.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= self.fovea.radius {
            FoveationLevel::Fovea
        } else if dist <= self.mid.radius {
            FoveationLevel::Mid
        } else {
            FoveationLevel::Peripheral
        }
    }

    /// Calculate the QP offset for a given pixel position.
    pub fn qp_offset_at(&self, px: f32, py: f32) -> i32 {
        match self.classify_pixel(px, py) {
            FoveationLevel::Fovea => self.fovea.qp_offset,
            FoveationLevel::Mid => self.mid.qp_offset,
            FoveationLevel::Peripheral => self.peripheral.qp_offset,
        }
    }

    /// Calculate the total bandwidth savings compared to uniform encoding.
    /// Returns a value like 0.40 meaning 40% bandwidth reduction.
    pub fn estimated_bandwidth_savings(&self) -> f32 {
        // Approximate area fractions (circular regions)
        let fovea_area = std::f32::consts::PI * self.fovea.radius * self.fovea.radius;
        let mid_area = std::f32::consts::PI * self.mid.radius * self.mid.radius - fovea_area;
        let peripheral_area = 1.0 - fovea_area - mid_area;

        // Quality reduction per region (QP offset → ~6% per QP step)
        let mid_reduction = 1.0 - (0.94_f32).powi(self.mid.qp_offset);
        let peripheral_reduction = 1.0 - (0.94_f32).powi(self.peripheral.qp_offset);

        // Weighted savings
        mid_area * mid_reduction + peripheral_area * peripheral_reduction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_gaze_is_center() {
        let config = FoveationConfig::default();
        assert_eq!(config.gaze.x, 0.5);
        assert_eq!(config.gaze.y, 0.5);
    }

    #[test]
    fn test_classify_center_pixel() {
        let config = FoveationConfig::default();
        assert_eq!(config.classify_pixel(0.5, 0.5), FoveationLevel::Fovea);
    }

    #[test]
    fn test_classify_edge_pixel() {
        let config = FoveationConfig::default();
        assert_eq!(config.classify_pixel(0.0, 0.0), FoveationLevel::Peripheral);
        assert_eq!(config.classify_pixel(1.0, 1.0), FoveationLevel::Peripheral);
    }

    #[test]
    fn test_classify_mid_pixel() {
        let config = FoveationConfig::default();
        // 0.25 away from center = within mid radius (0.35)
        assert_eq!(config.classify_pixel(0.75, 0.5), FoveationLevel::Mid);
    }

    #[test]
    fn test_update_gaze() {
        let mut config = FoveationConfig::default();
        config.update_gaze(0.3, 0.7);
        assert_eq!(config.gaze.x, 0.3);
        assert_eq!(config.gaze.y, 0.7);
        // Now center should be near the new gaze
        assert_eq!(config.classify_pixel(0.3, 0.7), FoveationLevel::Fovea);
    }

    #[test]
    fn test_bandwidth_savings() {
        let config = FoveationConfig::default();
        let savings = config.estimated_bandwidth_savings();
        // Should be roughly 30-50%
        assert!(savings > 0.2, "savings={}", savings);
        assert!(savings < 0.7, "savings={}", savings);
    }

    #[test]
    fn test_gaze_clamping() {
        let mut config = FoveationConfig::default();
        config.update_gaze(-0.5, 1.5);
        assert_eq!(config.gaze.x, 0.0);
        assert_eq!(config.gaze.y, 1.0);
    }
}
