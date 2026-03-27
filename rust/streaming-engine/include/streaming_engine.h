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
 * Submit pre-encoded H.265 NAL units for RTP packetization and transmission.
 * Called from the C++ driver after NVENC encoding.
 *
 * `nal_data_ptr`: pointer to encoded NAL byte array.
 * `nal_data_len`: length of the NAL data in bytes.
 * `frame_index`: monotonically increasing frame counter.
 * `is_idr`: 1 if this frame is an IDR keyframe, 0 otherwise.
 *
 * Returns 0 on success, -1 on error.
 */
int32_t fvp_submit_encoded_nal(const uint8_t *nal_data_ptr,
                               uint32_t nal_data_len,
                               uint32_t frame_index,
                               int32_t is_idr);

/**
 * Get the latest tracking data from the connected HMD.
 * Returns 0 on success, -1 if no data available.
 */
int32_t fvp_get_tracking_data(struct TrackingData *out);

/**
 * Get the latest controller state.
 * `controller_id`: 0 = left, 1 = right.
 * Returns 0 on success, -1 if no data available.
 */
int32_t fvp_get_controller_state(uint8_t controller_id, struct ControllerState *out);

/**
 * Get the display/video configuration.
 * Returns 0 on success, -1 if config not loaded.
 */
int32_t fvp_get_config(struct FvpConfig *out);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* FVP_STREAMING_ENGINE_H */
