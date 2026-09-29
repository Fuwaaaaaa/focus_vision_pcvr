#pragma once

#include <openxr/openxr.h>
#include "tracking_sender.h"

/// Polls OpenXR controller inputs (buttons, triggers, thumbsticks) and sends to PC.
///
/// The app owns the parts a session allows only once or wants once per
/// frame: it attaches every action set in one xrAttachSessionActionSets and
/// calls xrSyncActions once before pollAndSend.
class ControllerPoller {
public:
    /// Create the actions and suggest bindings: the Focus 3 controller
    /// profile when its extension is enabled (`focus3Profile`), the Khronos
    /// simple controller always. Returns the action set for the app to
    /// attach, or XR_NULL_HANDLE on failure.
    XrActionSet createActions(XrInstance instance, bool focus3Profile);

    /// After the app attached the action sets: the hand pose spaces.
    bool createSpaces(XrSession session);

    void shutdown();

    /// Set HMD battery level (0-100). Call from main app with JNI-obtained value.
    void setHmdBattery(uint8_t level) { m_hmdBattery = level; }

    /// Send each tracked controller's state via TrackingSender, as synced
    /// by the app's xrSyncActions this frame. Call every frame.
    void pollAndSend(XrSession session, XrSpace stageSpace,
                     XrTime predictedTime, TrackingSender& sender);

    /// Apply haptic vibration to a controller. Called when PC sends HAPTIC_EVENT.
    void applyHaptic(XrSession session, int hand, float durationSec, float frequency, float amplitude);

private:
    bool createActionSet(XrInstance instance);
    bool createActionsInSet();
    void suggestBindings(XrInstance instance, bool focus3Profile);

    XrActionSet m_actionSet = XR_NULL_HANDLE;

    // Actions
    XrAction m_poseAction = XR_NULL_HANDLE;     // Controller pose (6DoF)
    XrAction m_triggerAction = XR_NULL_HANDLE;   // Trigger (0-1)
    XrAction m_gripAction = XR_NULL_HANDLE;      // Grip (0-1)
    XrAction m_thumbstickAction = XR_NULL_HANDLE;// Thumbstick (x,y)
    XrAction m_aAction = XR_NULL_HANDLE;         // A/X button
    XrAction m_bAction = XR_NULL_HANDLE;         // B/Y button
    XrAction m_menuAction = XR_NULL_HANDLE;      // Menu button
    XrAction m_thumbstickClickAction = XR_NULL_HANDLE; // Thumbstick click
    XrAction m_triggerTouchAction = XR_NULL_HANDLE;    // Trigger touch
    XrAction m_gripTouchAction = XR_NULL_HANDLE;       // Grip touch
    XrAction m_thumbstickTouchAction = XR_NULL_HANDLE; // Thumbstick touch
    XrAction m_hapticAction = XR_NULL_HANDLE;    // Haptic output

    // Spaces for controller poses
    XrSpace m_handSpaces[2] = {XR_NULL_HANDLE, XR_NULL_HANDLE};

    // Subaction paths
    XrPath m_handPaths[2] = {0, 0};

    bool m_initialized = false;
    uint8_t m_hmdBattery = 100; // Updated from Android BatteryManager
};
