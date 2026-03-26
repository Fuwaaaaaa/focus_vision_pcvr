pub mod config;

use std::ffi::c_void;
use std::sync::Once;

use fvp_common::protocol::TrackingData;

static INIT: Once = Once::new();

/// Initialize the streaming engine. Returns 0 on success.
#[no_mangle]
pub extern "C" fn fvp_init() -> i32 {
    INIT.call_once(|| { env_logger::init(); });
    log::info!("Focus Vision PCVR Streaming Engine initialized");
    0
}

/// Shut down the streaming engine.
#[no_mangle]
pub extern "C" fn fvp_shutdown() {
    log::info!("Focus Vision PCVR Streaming Engine shutting down");
}

/// Submit a video frame for encoding and transmission.
/// Returns 0 on success.
#[no_mangle]
pub extern "C" fn fvp_submit_frame(
    _texture_ptr: *mut c_void,
    _width: u32,
    _height: u32,
    _frame_index: u32,
) -> i32 {
    0 // Stub — Step 5
}

/// Get the latest tracking data. Returns 0 on success, -1 if unavailable.
#[no_mangle]
pub extern "C" fn fvp_get_tracking_data(out: *mut TrackingData) -> i32 {
    if out.is_null() { return -1; }
    -1 // Stub — Step 6
}
