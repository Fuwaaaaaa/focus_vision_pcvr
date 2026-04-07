#pragma once

#include <openxr/openxr.h>
#include "tracking_sender.h"

/// Polls OpenXR controller inputs (buttons, triggers, thumbsticks) and sends to PC.
class ControllerPoller {
public:
    bool init(XrInstance instance, XrSession session);
    void shutdown();

    /// Poll controller state and send via TrackingSender. Call every frame.
    void pollAndSend(XrSession session, XrSpace stageSpace,
                     XrTime predictedTime, TrackingSender& sender);

    /// Apply haptic vibration to a controller. Called when PC sends HAPTIC_EVENT.
    void applyHaptic(XrSession session, int hand, float durationSec, float frequency, float amplitude);

private:
    bool createActionSet(XrInstance instance);
    bool createActions();
    bool suggestBindings(XrInstance instance);
    bool attachActionSet(XrSession session);

    XrActionSet m_actionSet = XR_NULL_HANDLE;

    // Actions
    XrAction m_poseAction = XR_NULL_HANDLE;     // Controller pose (6DoF)
    XrAction m_triggerAction = XR_NULL_HANDLE;   // Trigger (0-1)
    XrAction m_gripAction = XR_NULL_HANDLE;      // Grip (0-1)
    XrAction m_thumbstickAction = XR_NULL_HANDLE;// Thumbstick (x,y)
    XrAction m_aAction = XR_NULL_HANDLE;         // A/X button
    XrAction m_bAction = XR_NULL_HANDLE;         // B/Y button
    XrAction m_menuAction = XR_NULL_HANDLE;      // Menu button
    XrAction m_hapticAction = XR_NULL_HANDLE;    // Haptic output

    // Spaces for controller poses
    XrSpace m_handSpaces[2] = {XR_NULL_HANDLE, XR_NULL_HANDLE};

    // Subaction paths
    XrPath m_handPaths[2] = {0, 0};

    bool m_initialized = false;
};
