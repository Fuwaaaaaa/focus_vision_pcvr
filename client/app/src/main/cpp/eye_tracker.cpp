#include "eye_tracker.h"
#include "xr_utils.h"
#include <cstring>

XrActionSet EyeTracker::createActions(XrInstance instance, XrSystemId system, bool extensionEnabled) {
    if (!extensionEnabled) {
        LOGI("EyeTracker: XR_EXT_eye_gaze_interaction not available — using fixed center");
        return XR_NULL_HANDLE;
    }

    // The extension can be present on a system without eye tracking.
    XrSystemEyeGazeInteractionPropertiesEXT eyeGaze = {XR_TYPE_SYSTEM_EYE_GAZE_INTERACTION_PROPERTIES_EXT};
    XrSystemProperties properties = {XR_TYPE_SYSTEM_PROPERTIES};
    properties.next = &eyeGaze;
    if (XR_FAILED(xrGetSystemProperties(instance, system, &properties)) || !eyeGaze.supportsEyeGazeInteraction) {
        LOGI("EyeTracker: this headset has no eye gaze interaction — using fixed center");
        return XR_NULL_HANDLE;
    }

    // Create action set for eye gaze
    XrActionSetCreateInfo actionSetInfo = {XR_TYPE_ACTION_SET_CREATE_INFO};
    strncpy(actionSetInfo.actionSetName, "eye_gaze", XR_MAX_ACTION_SET_NAME_SIZE);
    strncpy(actionSetInfo.localizedActionSetName, "Eye Gaze", XR_MAX_LOCALIZED_ACTION_SET_NAME_SIZE);
    if (xrCreateActionSet(instance, &actionSetInfo, &m_actionSet) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to create action set");
        m_actionSet = XR_NULL_HANDLE;
        return XR_NULL_HANDLE;
    }

    // Create gaze action
    XrActionCreateInfo actionInfo = {XR_TYPE_ACTION_CREATE_INFO};
    actionInfo.actionType = XR_ACTION_TYPE_POSE_INPUT;
    strncpy(actionInfo.actionName, "gaze_pose", XR_MAX_ACTION_NAME_SIZE);
    strncpy(actionInfo.localizedActionName, "Gaze Pose", XR_MAX_LOCALIZED_ACTION_NAME_SIZE);
    if (xrCreateAction(m_actionSet, &actionInfo, &m_gazeAction) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to create gaze action");
        shutdown();
        return XR_NULL_HANDLE;
    }

    // Suggest interaction profile for eye gaze
    XrPath gazePath;
    xrStringToPath(instance, "/user/eyes_ext/input/gaze_ext/pose", &gazePath);

    XrPath interactionProfile;
    xrStringToPath(instance, "/interaction_profiles/ext/eye_gaze_interaction", &interactionProfile);

    XrActionSuggestedBinding binding = {m_gazeAction, gazePath};
    XrInteractionProfileSuggestedBinding suggestedBinding = {XR_TYPE_INTERACTION_PROFILE_SUGGESTED_BINDING};
    suggestedBinding.interactionProfile = interactionProfile;
    suggestedBinding.suggestedBindings = &binding;
    suggestedBinding.countSuggestedBindings = 1;

    if (xrSuggestInteractionProfileBindings(instance, &suggestedBinding) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to suggest gaze binding");
        shutdown();
        return XR_NULL_HANDLE;
    }
    return m_actionSet;
}

bool EyeTracker::createSpace(XrSession session, XrSpace viewSpace) {
    if (m_gazeAction == XR_NULL_HANDLE) return false;
    m_session = session;
    m_viewSpace = viewSpace;

    XrActionSpaceCreateInfo spaceInfo = {XR_TYPE_ACTION_SPACE_CREATE_INFO};
    spaceInfo.action = m_gazeAction;
    spaceInfo.poseInActionSpace.orientation.w = 1.0f;
    if (xrCreateActionSpace(session, &spaceInfo, &m_gazeSpace) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to create gaze space");
        return false;
    }

    m_available = true;
    LOGI("EyeTracker: initialized — eye tracking active");
    return true;
}

void EyeTracker::shutdown() {
    if (m_gazeSpace != XR_NULL_HANDLE) {
        xrDestroySpace(m_gazeSpace);
        m_gazeSpace = XR_NULL_HANDLE;
    }
    if (m_gazeAction != XR_NULL_HANDLE) {
        xrDestroyAction(m_gazeAction);
        m_gazeAction = XR_NULL_HANDLE;
    }
    if (m_actionSet != XR_NULL_HANDLE) {
        xrDestroyActionSet(m_actionSet);
        m_actionSet = XR_NULL_HANDLE;
    }
    m_available = false;
}

EyeTracker::GazeData EyeTracker::poll(XrTime displayTime, const fvp_video::Tangents& fov) {
    GazeData data = {0.5f, 0.5f, false, 0}; // Default: center

    if (!m_available || m_gazeSpace == XR_NULL_HANDLE) {
        return data;
    }

    // Get gaze pose state
    XrActionStatePose poseState = {XR_TYPE_ACTION_STATE_POSE};
    XrActionStateGetInfo getInfo = {XR_TYPE_ACTION_STATE_GET_INFO};
    getInfo.action = m_gazeAction;
    xrGetActionStatePose(m_session, &getInfo, &poseState);

    if (!poseState.isActive) {
        return data; // Eyes not tracked this frame
    }

    // Locate gaze in view space
    XrSpaceLocation location = {XR_TYPE_SPACE_LOCATION};
    if (xrLocateSpace(m_gazeSpace, m_viewSpace, displayTime, &location) != XR_SUCCESS) {
        return data;
    }

    if (!(location.locationFlags & XR_SPACE_LOCATION_ORIENTATION_VALID_BIT)) {
        return data;
    }

    // Where the gaze falls in the eye image (fvp_video::gazeInImage, tested).
    const XrQuaternionf& q = location.pose.orientation;
    fvp_video::gazeInImage({q.x, q.y, q.z, q.w}, fov, data.x, data.y);
    data.valid = true;
    data.timestamp_ns = (uint64_t)displayTime;

    return data;
}
