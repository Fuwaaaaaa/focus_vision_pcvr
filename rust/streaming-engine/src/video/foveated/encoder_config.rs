use super::region::{FoveationConfig, FoveationLevel};

/// NVENC encoder parameters adjusted per-region for foveated encoding.
///
/// These values are passed to the C++ NvencEncoder via C ABI to configure
/// per-region QP offsets in the NVENC ROI (Region of Interest) API.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FoveatedEncodeParams {
    /// Gaze position in pixels (from eye tracker or default center).
    pub gaze_x: u32,
    pub gaze_y: u32,
    /// Frame dimensions.
    pub frame_width: u32,
    pub frame_height: u32,
    /// Fovea region radius in pixels.
    pub fovea_radius_px: u32,
    /// Mid region radius in pixels.
    pub mid_radius_px: u32,
    /// QP offset for mid region (positive = lower quality).
    pub mid_qp_offset: i32,
    /// QP offset for peripheral region.
    pub peripheral_qp_offset: i32,
    /// Whether foveated encoding is active.
    pub enabled: i32,
}

impl FoveatedEncodeParams {
    /// Build NVENC-ready parameters from the foveation config and frame dimensions.
    pub fn from_config(config: &FoveationConfig, width: u32, height: u32) -> Self {
        if !config.enabled {
            return Self {
                gaze_x: width / 2,
                gaze_y: height / 2,
                frame_width: width,
                frame_height: height,
                fovea_radius_px: 0,
                mid_radius_px: 0,
                mid_qp_offset: 0,
                peripheral_qp_offset: 0,
                enabled: 0,
            };
        }

        Self {
            gaze_x: (config.gaze.x * width as f32) as u32,
            gaze_y: (config.gaze.y * height as f32) as u32,
            frame_width: width,
            frame_height: height,
            fovea_radius_px: (config.fovea.radius * width as f32) as u32,
            mid_radius_px: (config.mid.radius * width as f32) as u32,
            mid_qp_offset: config.mid.qp_offset,
            peripheral_qp_offset: config.peripheral.qp_offset,
            enabled: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_params() {
        let config = FoveationConfig::default(); // enabled=false
        let params = FoveatedEncodeParams::from_config(&config, 1832, 1920);
        assert_eq!(params.enabled, 0);
        assert_eq!(params.gaze_x, 916); // center
        assert_eq!(params.gaze_y, 960);
    }

    #[test]
    fn test_enabled_params() {
        let mut config = FoveationConfig::default();
        config.enabled = true;
        config.update_gaze(0.6, 0.4);
        let params = FoveatedEncodeParams::from_config(&config, 1832, 1920);
        assert_eq!(params.enabled, 1);
        assert_eq!(params.gaze_x, 1099); // 0.6 * 1832
        assert_eq!(params.gaze_y, 768);  // 0.4 * 1920
        assert_eq!(params.fovea_radius_px, 274); // 0.15 * 1832
        assert_eq!(params.mid_qp_offset, 5);
        assert_eq!(params.peripheral_qp_offset, 15);
    }
}
