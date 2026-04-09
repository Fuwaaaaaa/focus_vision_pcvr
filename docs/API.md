# Focus Vision PCVR — C FFI API Reference

The streaming engine exposes a C-compatible FFI for the OpenVR driver DLL.
All functions are `extern "C"` with `#[no_mangle]` — safe to call from C++.

## Lifecycle

### `fvp_init() -> i32`
Initialize the streaming engine. Loads `config/default.toml`, starts the tokio runtime, TCP listener, and tracking receiver.
- **Returns:** `0` on success, `-1` on error.
- **Thread safety:** Call once from the main thread. Subsequent calls return `-1`.

### `fvp_shutdown()`
Shut down the engine. Cancels all async tasks, closes TCP/UDP sockets, and drops state.
- **Thread safety:** Safe to call from any thread. No-op if not initialized.

## Callbacks

All callbacks are registered once during driver initialization.
They fire from the tokio runtime thread — keep handlers fast.

### `fvp_set_idr_callback(callback: extern "C" fn())`
Register a callback for IDR frame requests. Fires when the HMD client sends `IDR_REQUEST` over the TCP control channel. The driver should produce an IDR keyframe on the next encode cycle.

### `fvp_set_gaze_callback(callback: extern "C" fn(gaze_x: f32, gaze_y: f32, valid: i32))`
Register a callback for eye gaze updates from HMD tracking data.
- `gaze_x`, `gaze_y`: Normalized gaze coordinates (0.0 = left/top, 1.0 = right/bottom).
- `valid`: `1` if gaze data is valid, `0` if the tracker lost the eyes.
- Used by `NvencEncoder` to position the foveated encoding center point.

### `fvp_set_bitrate_callback(callback: extern "C" fn(bitrate_bps: u32))`
Register a callback for adaptive bitrate changes.
- `bitrate_bps`: New target bitrate in **bits per second** (not Mbps).
- Fires when: bandwidth estimator detects loss, sleep mode transitions, or HMD dashboard CONFIG_UPDATE.

## Frame Submission

### `fvp_submit_encoded_nal(nal_data_ptr: *const u8, nal_data_len: u32, frame_index: u32, is_idr: i32) -> i32`
Submit pre-encoded H.264/H.265 NAL units for RTP packetization and UDP transmission.
- `nal_data_ptr`: Pointer to encoded NAL byte array. **Must be valid for `nal_data_len` bytes.**
- `nal_data_len`: Length in bytes. Must be > 0.
- `frame_index`: Monotonically increasing frame counter.
- `is_idr`: `1` for IDR keyframes, `0` for P-frames.
- **Returns:** `0` on success, `-1` on error (engine not initialized, null pointer, channel full).
- **Safety:** `nal_data_ptr` must be valid. Null/zero-length checks are performed.
- **Performance:** Uses a thread-local buffer to avoid per-frame allocation.

## Data Queries

### `fvp_get_tracking_data(out: *mut TrackingData) -> i32`
Get the latest 6DoF head tracking data from the connected HMD.
- `out`: Pointer to `TrackingData` struct (defined in `fvp_common`).
- **Returns:** `0` on success, `-1` if no data available or engine not initialized.
- **Safety:** `out` must be a valid, aligned pointer.

### `fvp_get_controller_state(controller_id: u8, out: *mut ControllerState) -> i32`
Get the latest controller state.
- `controller_id`: `0` = left hand, `1` = right hand.
- `out`: Pointer to `ControllerState` struct.
- **Returns:** `0` on success, `-1` if no data or invalid ID.
- **Safety:** `out` must be a valid, aligned pointer.

### `fvp_get_config(out: *mut FvpConfig) -> i32`
Get the display/video configuration (resolution, refresh rate, IPD, foveated settings).
- `out`: Pointer to `FvpConfig` struct.
- **Returns:** `0` on success, `-1` if config not loaded.
- **Safety:** `out` must be a valid, aligned pointer.

## Haptic Feedback

### `fvp_haptic_event(controller_id: u8, duration_ms: u16, frequency: f32, amplitude: f32)`
Queue a haptic vibration event for delivery to the HMD controller via TCP.
- `controller_id`: `0` = left, `1` = right.
- `duration_ms`: Vibration duration in milliseconds.
- `frequency`: Vibration frequency in Hz.
- `amplitude`: Vibration intensity 0.0–1.0.
- **Thread safety:** Safe to call from any thread. Events are queued to an async channel.
- **Backpressure:** If the channel is full (16 events), the event is silently dropped and `HAPTIC_DROPS` counter increments.

## Struct Layouts

See `fvp_common::protocol` for `TrackingData` and `ControllerState` definitions.
See `lib.rs:FvpConfig` for the config struct layout.

All structs use `#[repr(C)]` for C ABI compatibility.
