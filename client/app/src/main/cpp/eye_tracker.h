#pragma once

#include <openxr/openxr.h>
#include <cstdint>
#include <atomic>

#include "video_view.h"

/**
 * Eye tracking via OpenXR XR_EXT_eye_gaze_interaction.
 *
 * Polls gaze direction each frame and converts to normalized
 * screen coordinates (0-1) for foveated encoding.
 *
 * Supported HMDs: VIVE Focus Vision, Quest Pro, Vision Pro.
 * On HMDs without eye tracking, isAvailable() returns false
 * and the system falls back to fixed-center foveation.
 *
 * The app attaches the action set together with the others (a session
 * allows one xrAttachSessionActionSets) and syncs actions once per frame.
 */
class EyeTracker {
public:
    struct GazeData {
        float x;         // 0.0 = left, 1.0 = right (normalized)
        float y;         // 0.0 = top, 1.0 = bottom (normalized)
        bool valid;      // false if gaze data is unreliable this frame
        uint64_t timestamp_ns;
    };

    /// Create the gaze action and suggest its binding, if the extension is
    /// enabled (`extensionEnabled`) and the system supports eye gaze.
    /// Returns the action set for the app to attach, or XR_NULL_HANDLE.
    XrActionSet createActions(XrInstance instance, XrSystemId system, bool extensionEnabled);

    /// After the app attached the action sets: the gaze space, located in
    /// `viewSpace`.
    bool createSpace(XrSession session, XrSpace viewSpace);

    /// Shut down and release resources.
    void shutdown();

    /// The current gaze, as synced by the app's xrSyncActions this frame,
    /// placed in the eye image whose field of view is `fov`. Call once per
    /// frame after xrLocateViews.
    GazeData poll(XrTime displayTime, const fvp_video::Tangents& fov);

    bool isAvailable() const { return m_available; }

private:
    bool m_available = false;
    XrSession m_session = XR_NULL_HANDLE;
    XrSpace m_gazeSpace = XR_NULL_HANDLE;
    XrSpace m_viewSpace = XR_NULL_HANDLE;
    XrActionSet m_actionSet = XR_NULL_HANDLE;
    XrAction m_gazeAction = XR_NULL_HANDLE;
};
