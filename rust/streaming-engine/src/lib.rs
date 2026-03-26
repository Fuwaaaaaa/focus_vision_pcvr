pub mod config;
pub mod video;
pub mod transport;
pub mod pipeline;
pub mod control;
pub mod metrics;
pub mod engine;
pub mod tracking;
pub mod adaptive;

use std::ffi::c_void;
use std::sync::{RwLock, Once};

use engine::{StreamingEngine, SubmittedFrame};
use fvp_common::protocol::{ControllerState, TrackingData};
use metrics::latency::FrameTimestamps;

static INIT: Once = Once::new();
static ENGINE: RwLock<Option<StreamingEngine>> = RwLock::new(None);
static CONFIG: RwLock<Option<config::AppConfig>> = RwLock::new(None);

/// Configuration values exported to C++ driver.
#[repr(C)]
pub struct FvpConfig {
    pub render_width: u32,
    pub render_height: u32,
    pub refresh_rate: f32,
    pub ipd: f32,
    pub seconds_from_vsync_to_photons: f32,
}

/// Initialize the streaming engine. Returns 0 on success.
#[no_mangle]
pub extern "C" fn fvp_init() -> i32 {
    INIT.call_once(|| { env_logger::init(); });
    log::info!("Focus Vision PCVR Streaming Engine initializing...");

    let config = config::AppConfig::load("config/default.toml").unwrap_or_else(|e| {
        log::warn!("Failed to load config, using defaults: {}", e);
        config::AppConfig::default()
    });

    // Store config for fvp_get_config()
    if let Ok(mut guard) = CONFIG.write() {
        *guard = Some(config.clone());
    }
    match StreamingEngine::new(config) {
        Ok(eng) => {
            if let Ok(mut guard) = ENGINE.write() {
                *guard = Some(eng);
            }
            log::info!("Streaming engine started");
            0
        }
        Err(e) => {
            log::error!("Failed to start engine: {}", e);
            -1
        }
    }
}

/// Shut down the streaming engine.
#[no_mangle]
pub extern "C" fn fvp_shutdown() {
    log::info!("Focus Vision PCVR Streaming Engine shutting down");
    // Cancel all async tasks before dropping the engine
    if let Ok(guard) = ENGINE.read() {
        if let Some(engine) = guard.as_ref() {
            engine.shutdown();
        }
    }
    if let Ok(mut guard) = ENGINE.write() {
        *guard = None; // Drop the engine, which drops the runtime
    }
}

/// Submit a video frame for encoding and transmission.
/// `texture_ptr`: platform-specific handle (D3D11 texture on Windows).
/// `width`, `height`: frame dimensions.
/// `frame_index`: monotonically increasing frame counter.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn fvp_submit_frame(
    _texture_ptr: *mut c_void,
    width: u32,
    height: u32,
    frame_index: u32,
) -> i32 {
    let guard = match ENGINE.read() {
        Ok(g) => g,
        Err(e) => { log::error!("RwLock poisoned: {}", e); return -1; }
    };
    let engine = match guard.as_ref() {
        Some(e) => e,
        None => return -1,
    };

    // In production (Step 5 full): read D3D11 texture pixels here.
    // For now: create a placeholder frame to test the pipeline.
    let timestamps = FrameTimestamps::new(frame_index);
    let placeholder_data = video::test_pattern::generate_nv12_frame(width, height, frame_index as u64);

    let frame = SubmittedFrame {
        frame_index,
        data: placeholder_data,
        width,
        height,
        timestamps,
    };

    if engine.submit_frame(frame) { 0 } else { -1 }
}

/// Get the latest tracking data from the connected HMD.
/// Returns 0 on success, -1 if no data available.
#[no_mangle]
pub extern "C" fn fvp_get_tracking_data(out: *mut TrackingData) -> i32 {
    if out.is_null() {
        return -1;
    }

    let guard = match ENGINE.read() {
        Ok(g) => g,
        Err(e) => { log::error!("RwLock poisoned: {}", e); return -1; }
    };
    let engine = match guard.as_ref() {
        Some(e) => e,
        None => return -1,
    };

    match engine.get_tracking() {
        Some(data) => {
            unsafe { *out = data; }
            0
        }
        None => -1,
    }
}

/// Get the latest controller state.
/// `controller_id`: 0 = left, 1 = right.
/// Returns 0 on success, -1 if no data available.
#[no_mangle]
pub extern "C" fn fvp_get_controller_state(controller_id: u8, out: *mut ControllerState) -> i32 {
    if out.is_null() {
        return -1;
    }

    let guard = match ENGINE.read() {
        Ok(g) => g,
        Err(e) => { log::error!("RwLock poisoned: {}", e); return -1; }
    };
    let engine = match guard.as_ref() {
        Some(e) => e,
        None => return -1,
    };

    match engine.get_controller(controller_id) {
        Some(state) => {
            unsafe { *out = state; }
            0
        }
        None => -1,
    }
}

/// Get the display/video configuration.
/// Returns 0 on success, -1 if config not loaded.
#[no_mangle]
pub extern "C" fn fvp_get_config(out: *mut FvpConfig) -> i32 {
    if out.is_null() {
        return -1;
    }

    let guard = match CONFIG.read() {
        Ok(g) => g,
        Err(e) => { log::error!("RwLock poisoned: {}", e); return -1; }
    };
    let cfg = match guard.as_ref() {
        Some(c) => c,
        None => return -1,
    };

    unsafe {
        (*out).render_width = cfg.video.resolution_per_eye[0];
        (*out).render_height = cfg.video.resolution_per_eye[1];
        (*out).refresh_rate = cfg.video.framerate as f32;
        (*out).ipd = cfg.display.ipd;
        (*out).seconds_from_vsync_to_photons = cfg.display.seconds_from_vsync_to_photons;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    // Reset engine state between tests — tests must be run serially
    fn reset_engine() {
        if let Ok(guard) = ENGINE.read() {
            if let Some(engine) = guard.as_ref() {
                engine.shutdown();
            }
        }
        if let Ok(mut guard) = ENGINE.write() {
            *guard = None;
        }
        if let Ok(mut guard) = CONFIG.write() {
            *guard = None;
        }
    }

    #[test]
    fn test_fvp_get_tracking_null_ptr() {
        let result = fvp_get_tracking_data(ptr::null_mut());
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_controller_null_ptr() {
        let result = fvp_get_controller_state(0, ptr::null_mut());
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_controller_invalid_id() {
        // Without engine, should return -1 regardless of id
        reset_engine();
        let mut state = ControllerState {
            controller_id: 0,
            timestamp_ns: 0,
            position: [0.0; 3],
            orientation: [0.0; 4],
            trigger: 0.0,
            grip: 0.0,
            thumbstick_x: 0.0,
            thumbstick_y: 0.0,
            button_flags: 0,
            battery_level: 0,
        };
        let result = fvp_get_controller_state(255, &mut state);
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_tracking_no_engine() {
        reset_engine();
        let mut data = TrackingData {
            position: [0.0; 3],
            orientation: [0.0; 4],
            timestamp_ns: 0,
        };
        let result = fvp_get_tracking_data(&mut data);
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_config_null_ptr() {
        let result = fvp_get_config(ptr::null_mut());
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_config_no_config() {
        reset_engine();
        let mut cfg = FvpConfig {
            render_width: 0,
            render_height: 0,
            refresh_rate: 0.0,
            ipd: 0.0,
            seconds_from_vsync_to_photons: 0.0,
        };
        let result = fvp_get_config(&mut cfg);
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_shutdown_without_init() {
        reset_engine();
        // Should not panic
        fvp_shutdown();
    }

    #[test]
    fn test_fvp_submit_frame_no_engine() {
        reset_engine();
        let result = fvp_submit_frame(ptr::null_mut(), 100, 100, 0);
        assert_eq!(result, -1);
    }
}
