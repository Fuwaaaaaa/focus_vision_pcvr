pub mod config;
pub mod video;
pub mod audio;
pub mod transport;
pub mod pipeline;
pub mod control;
pub mod metrics;
pub mod engine;
pub mod tracking;
pub mod adaptive;
pub mod codec_benchmark;
pub mod face_tracking;
pub mod sleep_mode;
pub mod recording;

use std::sync::{RwLock, Once};

use engine::{StreamingEngine, EncodedFrame};
use fvp_common::protocol::{ControllerState, TrackingData};
use metrics::latency::FrameTimestamps;

/// Subsystem status for the companion app.
#[derive(Debug, Default)]
pub struct SubsystemStatus {
    pub ft_active: bool,
    pub sleep_active: bool,
    pub audio_enabled: bool,
    pub packet_loss_pct: f32,
}

/// Write engine status to a shared JSON file for the companion app.
/// Path: %APPDATA%/FocusVisionPCVR/status.json (Windows)
/// Uses atomic write (temp file + rename) to prevent partial reads.
pub fn write_status_file(
    status: &str,
    pin: Option<u32>,
    latency_us: Option<u64>,
    fps: Option<u16>,
    bitrate_mbps: Option<u32>,
    subsystems: Option<&SubsystemStatus>,
) {
    let dir = match dirs_next::data_dir() {
        Some(d) => d.join("FocusVisionPCVR"),
        None => return,
    };
    let _ = std::fs::create_dir_all(&dir);
    let pin_str = pin.map(|p| format!("{:06}", p)).unwrap_or_else(|| "------".to_string());
    let sub = subsystems.map(|s| format!(
        r#","ft_active":{},"sleep_active":{},"audio_enabled":{},"packet_loss_pct":{:.1}"#,
        s.ft_active, s.sleep_active, s.audio_enabled, s.packet_loss_pct,
    )).unwrap_or_default();
    let json = format!(
        r#"{{"status":"{}","pin":"{}","latency_us":{},"fps":{},"bitrate_mbps":{}{}}}"#,
        status, pin_str,
        latency_us.unwrap_or(0),
        fps.unwrap_or(0),
        bitrate_mbps.unwrap_or(0),
        sub,
    );
    // Atomic write: write to temp file then rename to prevent partial reads
    let path = dir.join("status.json");
    let tmp_path = dir.join("status.json.tmp");
    if std::fs::write(&tmp_path, &json).is_ok() {
        let _ = std::fs::rename(&tmp_path, &path);
    }
}

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
    // Foveated encoding settings from TOML config
    pub full_range: i32,
    pub foveated_enabled: i32,
    pub fovea_radius: f32,
    pub mid_radius: f32,
    pub mid_qp_offset: i32,
    pub peripheral_qp_offset: i32,
}

/// Initialize the streaming engine. Returns 0 on success.
#[no_mangle]
pub extern "C" fn fvp_init() -> i32 {
    INIT.call_once(|| { env_logger::init(); });
    log::info!("Focus Vision PCVR Streaming Engine initializing...");

    let mut config = config::AppConfig::load("config/default.toml").unwrap_or_else(|e| {
        log::warn!("Failed to load config, using defaults: {}", e);
        config::AppConfig::default()
    });

    // Validate and fix invalid config values (graceful: clamp + warn)
    let errors = config.validate();
    for e in &errors {
        log::warn!("{}", e);
    }

    // Store config for fvp_get_config()
    if let Ok(mut guard) = CONFIG.write() {
        *guard = Some(config.clone());
    }
    match StreamingEngine::new(config) {
        Ok(eng) => {
            if let Ok(mut guard) = ENGINE.write() {
                *guard = Some(eng);
            }
            write_status_file("waiting", None, None, None, None, None);
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
    // Single write lock: shutdown + drop atomically to avoid race conditions
    if let Ok(mut guard) = ENGINE.write() {
        if let Some(engine) = guard.take() {
            engine.shutdown();
            // engine is dropped here when it goes out of scope
        }
    }
}

/// Register an IDR request callback. Called from C++ on init.
/// When the HMD sends an IDR_REQUEST over TCP, this callback fires
/// so the C++ NVENC encoder can produce an IDR frame.
#[no_mangle]
pub extern "C" fn fvp_set_idr_callback(callback: extern "C" fn()) {
    engine::set_idr_callback(callback);
    log::info!("IDR callback registered");
}

/// Register a gaze update callback. Called from C++ on init.
/// When the HMD sends tracking data with gaze info, this callback
/// forwards gaze coordinates to the C++ NVENC encoder for foveated encoding.
#[no_mangle]
pub extern "C" fn fvp_set_gaze_callback(callback: extern "C" fn(f32, f32, i32)) {
    engine::set_gaze_callback(callback);
    log::info!("Gaze callback registered");
}

/// Register a bitrate change callback. Called from C++ on init.
/// When adaptive bitrate adjusts the target, this callback fires
/// so the C++ NVENC encoder can reconfigure its bitrate.
/// `bitrate_bps`: new target bitrate in bits per second.
#[no_mangle]
pub extern "C" fn fvp_set_bitrate_callback(callback: extern "C" fn(u32)) {
    engine::set_bitrate_callback(callback);
    log::info!("Bitrate callback registered");
}

/// Queue a haptic vibration event for delivery to HMD controller.
/// Called from C++ OpenVR driver when SteamVR requests haptic feedback.
/// `controller_id`: 0=left, 1=right.
/// `duration_ms`: vibration duration in milliseconds.
/// `frequency`: vibration frequency in Hz.
/// `amplitude`: vibration intensity 0.0-1.0.
#[no_mangle]
pub extern "C" fn fvp_haptic_event(
    controller_id: u8,
    duration_ms: u16,
    frequency: f32,
    amplitude: f32,
) {
    engine::queue_haptic(controller_id, duration_ms, frequency, amplitude);
}

/// Submit pre-encoded H.265 NAL units for RTP packetization and transmission.
/// Called from the C++ driver after NVENC encoding.
///
/// `nal_data_ptr`: pointer to encoded NAL byte array.
/// `nal_data_len`: length of the NAL data in bytes.
/// `frame_index`: monotonically increasing frame counter.
/// `is_idr`: 1 if this frame is an IDR keyframe, 0 otherwise.
///
/// Returns 0 on success, -1 on error.
///
/// # Safety
/// `nal_data_ptr` must be valid for `nal_data_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn fvp_submit_encoded_nal(
    nal_data_ptr: *const u8,
    nal_data_len: u32,
    frame_index: u32,
    is_idr: i32,
) -> i32 {
    if nal_data_ptr.is_null() || nal_data_len == 0 {
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

    // Reuse a thread-local buffer to avoid per-frame allocation.
    // The Vec retains its capacity across calls, so after the first large frame,
    // subsequent frames reuse the same heap memory.
    // std::mem::take() transfers ownership without copying — the thread-local
    // is left empty (capacity 0) and will be replenished on the next call.
    thread_local! {
        static NAL_BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::with_capacity(256 * 1024));
    }

    // SAFETY: Caller (C++ driver) guarantees nal_data_ptr is valid for nal_data_len bytes.
    // Null check is performed above.
    let nal_slice = unsafe { std::slice::from_raw_parts(nal_data_ptr, nal_data_len as usize) };

    // Tap the raw NAL for session recording (no-op when disabled).
    engine.write_recording_nal(nal_slice);

    let nal_data = NAL_BUF.with(|buf| {
        let mut b = buf.borrow_mut();
        b.clear();
        b.extend_from_slice(nal_slice);
        std::mem::take(&mut *b)
    });

    let timestamps = FrameTimestamps::new(frame_index);

    let frame = EncodedFrame {
        frame_index,
        nal_data,
        is_idr: is_idr != 0,
        timestamps,
    };

    if engine.submit_frame(frame) { 0 } else { -1 }
}

/// Get the latest tracking data from the connected HMD.
/// Returns 0 on success, -1 if no data available.
///
/// # Safety
/// `out` must be a valid, aligned pointer to TrackingData.
#[no_mangle]
pub unsafe extern "C" fn fvp_get_tracking_data(out: *mut TrackingData) -> i32 {
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
            // SAFETY: Null check above guarantees out is valid.
            unsafe { out.write(data); }
            0
        }
        None => -1,
    }
}

/// Get the latest controller state.
/// `controller_id`: 0 = left, 1 = right.
/// Returns 0 on success, -1 if no data available.
///
/// # Safety
/// `out` must be a valid, aligned pointer to ControllerState.
#[no_mangle]
pub unsafe extern "C" fn fvp_get_controller_state(controller_id: u8, out: *mut ControllerState) -> i32 {
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
            out.write(state);
            0
        }
        None => -1,
    }
}

/// Get the display/video configuration.
/// Returns 0 on success, -1 if config not loaded.
///
/// # Safety
/// `out` must be a valid, aligned pointer to FvpConfig.
#[no_mangle]
pub unsafe extern "C" fn fvp_get_config(out: *mut FvpConfig) -> i32 {
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

    out.write(FvpConfig {
        render_width: cfg.video.resolution_per_eye[0],
        render_height: cfg.video.resolution_per_eye[1],
        refresh_rate: cfg.video.framerate as f32,
        ipd: cfg.display.ipd,
        seconds_from_vsync_to_photons: cfg.display.seconds_from_vsync_to_photons,
        full_range: if cfg.video.full_range { 1 } else { 0 },
        foveated_enabled: if cfg.foveated.enabled { 1 } else { 0 },
        fovea_radius: cfg.foveated.fovea_radius,
        mid_radius: cfg.foveated.mid_radius,
        mid_qp_offset: {
            let (mid, _) = cfg.foveated.effective_qp_offsets();
            mid
        },
        peripheral_qp_offset: {
            let (_, periph) = cfg.foveated.effective_qp_offsets();
            periph
        },
    });
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
        let result = unsafe { fvp_get_tracking_data(ptr::null_mut()) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_controller_null_ptr() {
        let result = unsafe { fvp_get_controller_state(0, ptr::null_mut()) };
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
        let result = unsafe { fvp_get_controller_state(255, &mut state) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_tracking_no_engine() {
        reset_engine();
        let mut data = TrackingData {
            position: [0.0; 3],
            orientation: [0.0; 4],
            timestamp_ns: 0,
            gaze_x: 0.5,
            gaze_y: 0.5,
            gaze_valid: 0,
        };
        let result = unsafe { fvp_get_tracking_data(&mut data) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_get_config_null_ptr() {
        let result = unsafe { fvp_get_config(ptr::null_mut()) };
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
            full_range: 0,
            foveated_enabled: 0,
            fovea_radius: 0.0,
            mid_radius: 0.0,
            mid_qp_offset: 0,
            peripheral_qp_offset: 0,
        };
        let result = unsafe { fvp_get_config(&mut cfg) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_shutdown_without_init() {
        reset_engine();
        // Should not panic
        fvp_shutdown();
    }

    #[test]
    fn test_fvp_submit_encoded_nal_no_engine() {
        reset_engine();
        let nal_data = vec![0u8; 100];
        let result = unsafe { fvp_submit_encoded_nal(nal_data.as_ptr(), nal_data.len() as u32, 0, 1) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_submit_encoded_nal_null_ptr() {
        reset_engine();
        let result = unsafe { fvp_submit_encoded_nal(ptr::null(), 100, 0, 0) };
        assert_eq!(result, -1);
    }

    #[test]
    fn test_fvp_submit_encoded_nal_zero_len() {
        reset_engine();
        let nal_data = vec![0u8; 100];
        let result = unsafe { fvp_submit_encoded_nal(nal_data.as_ptr(), 0, 0, 0) };
        assert_eq!(result, -1);
    }
}
