pub mod config;
pub mod video;
pub mod transport;
pub mod pipeline;
pub mod control;
pub mod metrics;
pub mod engine;

use std::ffi::c_void;
use std::sync::{Mutex, Once};

use engine::{StreamingEngine, SubmittedFrame};
use fvp_common::protocol::TrackingData;
use metrics::latency::FrameTimestamps;

static INIT: Once = Once::new();
static ENGINE: Mutex<Option<StreamingEngine>> = Mutex::new(None);

/// Initialize the streaming engine. Returns 0 on success.
#[no_mangle]
pub extern "C" fn fvp_init() -> i32 {
    INIT.call_once(|| { env_logger::init(); });
    log::info!("Focus Vision PCVR Streaming Engine initializing...");

    let config = config::AppConfig::default();
    match StreamingEngine::new(config) {
        Ok(eng) => {
            if let Ok(mut guard) = ENGINE.lock() {
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
    if let Ok(mut guard) = ENGINE.lock() {
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
    let guard = match ENGINE.lock() {
        Ok(g) => g,
        Err(_) => return -1,
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

    let guard = match ENGINE.lock() {
        Ok(g) => g,
        Err(_) => return -1,
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
