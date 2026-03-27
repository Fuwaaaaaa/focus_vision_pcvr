#include "eye_tracker.h"
#include "xr_utils.h"
#include <cstring>
#include <cmath>

bool EyeTracker::init(XrInstance instance, XrSession session, XrSpace viewSpace) {
    m_session = session;
    m_viewSpace = viewSpace;

    // Check if XR_EXT_eye_gaze_interaction is supported
    uint32_t extCount = 0;
    xrEnumerateInstanceExtensionProperties(nullptr, 0, &extCount, nullptr);
    std::vector<XrExtensionProperties> exts(extCount, {XR_TYPE_EXTENSION_PROPERTIES});
    xrEnumerateInstanceExtensionProperties(nullptr, extCount, &extCount, exts.data());

    bool hasEyeGaze = false;
    for (const auto& ext : exts) {
        if (strcmp(ext.extensionName, "XR_EXT_eye_gaze_interaction") == 0) {
            hasEyeGaze = true;
            break;
        }
    }

    if (!hasEyeGaze) {
        LOGI("EyeTracker: XR_EXT_eye_gaze_interaction not available — using fixed center");
        m_available = false;
        return false;
    }

    // Create action set for eye gaze
    XrActionSetCreateInfo actionSetInfo = {XR_TYPE_ACTION_SET_CREATE_INFO};
    strncpy(actionSetInfo.actionSetName, "eye_gaze", XR_MAX_ACTION_SET_NAME_SIZE);
    strncpy(actionSetInfo.localizedActionSetName, "Eye Gaze", XR_MAX_LOCALIZED_ACTION_SET_NAME_SIZE);
    if (xrCreateActionSet(instance, &actionSetInfo, &m_actionSet) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to create action set");
        return false;
    }

    // Create gaze action
    XrActionCreateInfo actionInfo = {XR_TYPE_ACTION_CREATE_INFO};
    actionInfo.actionType = XR_ACTION_TYPE_POSE_INPUT;
    strncpy(actionInfo.actionName, "gaze_pose", XR_MAX_ACTION_NAME_SIZE);
    strncpy(actionInfo.localizedActionName, "Gaze Pose", XR_MAX_LOCALIZED_ACTION_NAME_SIZE);
    if (xrCreateAction(m_actionSet, &actionInfo, &m_gazeAction) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to create gaze action");
        return false;
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
        return false;
    }

    // Create gaze space
    XrActionSpaceCreateInfo spaceInfo = {XR_TYPE_ACTION_SPACE_CREATE_INFO};
    spaceInfo.action = m_gazeAction;
    spaceInfo.poseInActionSpace.orientation.w = 1.0f;

    if (xrCreateActionSpace(session, &spaceInfo, &m_gazeSpace) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to create gaze space");
        return false;
    }

    // Attach action set to session
    XrSessionActionSetsAttachInfo attachInfo = {XR_TYPE_SESSION_ACTION_SETS_ATTACH_INFO};
    attachInfo.actionSets = &m_actionSet;
    attachInfo.countActionSets = 1;

    if (xrAttachSessionActionSets(session, &attachInfo) != XR_SUCCESS) {
        LOGW("EyeTracker: Failed to attach action set");
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

EyeTracker::GazeData EyeTracker::poll(XrTime displayTime) {
    GazeData data = {0.5f, 0.5f, false, 0}; // Default: center

    if (!m_available || m_gazeSpace == XR_NULL_HANDLE) {
        return data;
    }

    // Sync action set
    XrActiveActionSet activeSet = {m_actionSet, XR_NULL_PATH};
    XrActionsSyncInfo syncInfo = {XR_TYPE_ACTIONS_SYNC_INFO};
    syncInfo.activeActionSets = &activeSet;
    syncInfo.countActiveActionSets = 1;
    xrSyncActions(m_session, &syncInfo);

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

    // Convert gaze direction (quaternion → normalized screen coords)
    // The gaze pose orientation represents where the user is looking.
    // We need to project this onto the screen plane.
    //
    // Extract forward vector from quaternion:
    float qx = location.pose.orientation.x;
    float qy = location.pose.orientation.y;
    float qz = location.pose.orientation.z;
    float qw = location.pose.orientation.w;

    // Forward vector (0,0,-1) rotated by quaternion
    float fx = 2.0f * (qx * qz + qw * qy);
    float fy = 2.0f * (qy * qz - qw * qx);
    float fz = 1.0f - 2.0f * (qx * qx + qy * qy);

    // Project onto screen plane (assuming ~100 degree FOV)
    // x: positive = right, y: positive = down
    if (fz < 0.01f) fz = 0.01f; // Avoid division by zero
    float screenX = fx / fz;
    float screenY = -fy / fz; // Flip Y (screen Y is down)

    // Map from tangent space to normalized 0-1 coords
    // tan(50 degrees) ≈ 1.19 for ~100 degree FOV
    float halfFov = 1.19f;
    data.x = (screenX / halfFov + 1.0f) * 0.5f;
    data.y = (screenY / halfFov + 1.0f) * 0.5f;

    // Clamp to valid range
    data.x = std::fmax(0.0f, std::fmin(1.0f, data.x));
    data.y = std::fmax(0.0f, std::fmin(1.0f, data.y));
    data.valid = true;
    data.timestamp_ns = (uint64_t)displayTime;

    return data;
}
