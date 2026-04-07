#ifndef FVP_STREAMING_ENGINE_H
#define FVP_STREAMING_ENGINE_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Tracking data sent from HMD to PC
 */
typedef struct TrackingData {
    float position[3];
    float orientation[4];
    uint64_t timestamp_ns;
    /**
     * Eye gaze normalized coords (0-1). x=0.5,y=0.5 means center.
     * gaze_valid=0 means no eye tracking data (use center fallback).
     */
    float gaze_x;
    float gaze_y;
    uint8_t gaze_valid;
} TrackingData;

/**
 * Controller state sent from HMD to PC
 */
typedef struct ControllerState {
    uint8_t controller_id;
    uint64_t timestamp_ns;
    float position[3];
    float orientation[4];
    float trigger;
    float grip;
    float thumbstick_x;
    float thumbstick_y;
    uint32_t button_flags;
    uint8_t battery_level;
} ControllerState;

/**
 * Configuration values exported to C++ driver.
 */
typedef struct FvpConfig {
    uint32_t render_width;
    uint32_t render_height;
    float refresh_rate;
    float ipd;
    float seconds_from_vsync_to_photons;
    int32_t foveated_enabled;
    float fovea_radius;
    float mid_radius;
    int32_t mid_qp_offset;
    int32_t peripheral_qp_offset;
} FvpConfig;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Initialize the streaming engine. Returns 0 on success.
 */
int32_t fvp_init(void);

/**
 * Shut down the streaming engine.
 */
void fvp_shutdown(void);

/**
 * Register an IDR request callback. Called from C++ on init.
 * When the HMD sends an IDR_REQUEST over TCP, this callback fires
 * so the C++ NVENC encoder can produce an IDR frame.
 */
void fvp_set_idr_callback(void (*callback)(void));

/**
 * Register a gaze update callback. Called from C++ on init.
 * When the HMD sends tracking data with gaze info, this callback
 * forwards gaze coordinates to the C++ NVENC encoder for foveated encoding.
 */
void fvp_set_gaze_callback(void (*callback)(float, float, int32_t));

/**
 * Register a bitrate change callback. Called from C++ on init.
 * When adaptive bitrate adjusts the target, this callback fires
 * so the C++ NVENC encoder can reconfigure its bitrate.
 * `bitrate_bps`: new target bitrate in bits per second.
 */
void fvp_set_bitrate_callback(void (*callback)(uint32_t));

/**
 * Queue a haptic vibration event for delivery to HMD controller.
 * Called from C++ OpenVR driver when SteamVR requests haptic feedback.
 * `controller_id`: 0=left, 1=right.
 * `duration_ms`: vibration duration in milliseconds.
 * `frequency`: vibration frequency in Hz.
 * `amplitude`: vibration intensity 0.0-1.0.
 */
void fvp_haptic_event(uint8_t controller_id,
                      uint16_t duration_ms,
                      float frequency,
                      float amplitude);

/**
 * Submit pre-encoded H.265 NAL units for RTP packetization and transmission.
 * Called from the C++ driver after NVENC encoding.
 *
 * `nal_data_ptr`: pointer to encoded NAL byte array.
 * `nal_data_len`: length of the NAL data in bytes.
 * `frame_index`: monotonically increasing frame counter.
 * `is_idr`: 1 if this frame is an IDR keyframe, 0 otherwise.
 *
 * Returns 0 on success, -1 on error.
 *
 * # Safety
 * `nal_data_ptr` must be valid for `nal_data_len` bytes.
 */
int32_t fvp_submit_encoded_nal(const uint8_t *nal_data_ptr,
                               uint32_t nal_data_len,
                               uint32_t frame_index,
                               int32_t is_idr);

/**
 * Get the latest tracking data from the connected HMD.
 * Returns 0 on success, -1 if no data available.
 *
 * # Safety
 * `out` must be a valid, aligned pointer to TrackingData.
 */
int32_t fvp_get_tracking_data(struct TrackingData *out);

/**
 * Get the latest controller state.
 * `controller_id`: 0 = left, 1 = right.
 * Returns 0 on success, -1 if no data available.
 *
 * # Safety
 * `out` must be a valid, aligned pointer to ControllerState.
 */
int32_t fvp_get_controller_state(uint8_t controller_id, struct ControllerState *out);

/**
 * Get the display/video configuration.
 * Returns 0 on success, -1 if config not loaded.
 *
 * # Safety
 * `out` must be a valid, aligned pointer to FvpConfig.
 */
int32_t fvp_get_config(struct FvpConfig *out);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* FVP_STREAMING_ENGINE_H */
