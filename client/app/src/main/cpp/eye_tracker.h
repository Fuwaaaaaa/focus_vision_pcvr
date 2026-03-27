#pragma once

#include <openxr/openxr.h>
#include <cstdint>
#include <atomic>

/**
 * Eye tracking via OpenXR XR_EXT_eye_gaze_interaction.
 *
 * Polls gaze direction each frame and converts to normalized
 * screen coordinates (0-1) for foveated encoding.
 *
 * Supported HMDs: VIVE Focus Vision, Quest Pro, Vision Pro.
 * On HMDs without eye tracking, isAvailable() returns false
 * and the system falls back to fixed-center foveation.
 */
class EyeTracker {
public:
    struct GazeData {
        float x;         // 0.0 = left, 1.0 = right (normalized)
        float y;         // 0.0 = top, 1.0 = bottom (normalized)
        bool valid;      // false if gaze data is unreliable this frame
        uint64_t timestamp_ns;
    };

    /// Initialize eye tracking. Returns true if the extension is available.
    bool init(XrInstance instance, XrSession session, XrSpace viewSpace);

    /// Shut down and release resources.
    void shutdown();

    /// Poll current gaze. Call once per frame after xrLocateViews.
    GazeData poll(XrTime displayTime);

    bool isAvailable() const { return m_available; }

private:
    bool m_available = false;
    XrSession m_session = XR_NULL_HANDLE;
    XrSpace m_gazeSpace = XR_NULL_HANDLE;
    XrSpace m_viewSpace = XR_NULL_HANDLE;
    XrActionSet m_actionSet = XR_NULL_HANDLE;
    XrAction m_gazeAction = XR_NULL_HANDLE;
};
