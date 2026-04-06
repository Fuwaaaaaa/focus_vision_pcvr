#include "controller_poller.h"
#include "xr_utils.h"
#include <cstring>

bool ControllerPoller::init(XrInstance instance, XrSession session) {
    if (!createActionSet(instance)) return false;
    if (!createActions()) return false;
    if (!suggestBindings(instance)) return false;
    if (!attachActionSet(session)) return false;

    // Create hand spaces for pose tracking
    for (int i = 0; i < 2; i++) {
        XrActionSpaceCreateInfo spaceInfo = {XR_TYPE_ACTION_SPACE_CREATE_INFO};
        spaceInfo.action = m_poseAction;
        spaceInfo.subactionPath = m_handPaths[i];
        spaceInfo.poseInActionSpace.orientation.w = 1.0f;
        XR_CHECK(xrCreateActionSpace(session, &spaceInfo, &m_handSpaces[i]),
            "xrCreateActionSpace for hand");
    }

    m_initialized = true;
    LOGI("ControllerPoller initialized");
    return true;
}

void ControllerPoller::shutdown() {
    for (auto& space : m_handSpaces) {
        if (space != XR_NULL_HANDLE) { xrDestroySpace(space); space = XR_NULL_HANDLE; }
    }
    if (m_actionSet != XR_NULL_HANDLE) {
        xrDestroyActionSet(m_actionSet);
        m_actionSet = XR_NULL_HANDLE;
    }
    m_initialized = false;
}

bool ControllerPoller::createActionSet(XrInstance instance) {
    xrStringToPath(instance, "/user/hand/left", &m_handPaths[0]);
    xrStringToPath(instance, "/user/hand/right", &m_handPaths[1]);

    XrActionSetCreateInfo info = {XR_TYPE_ACTION_SET_CREATE_INFO};
    strncpy(info.actionSetName, "fvp_input", XR_MAX_ACTION_SET_NAME_SIZE);
    strncpy(info.localizedActionSetName, "FVP Input", XR_MAX_LOCALIZED_ACTION_SET_NAME_SIZE);
    info.priority = 0;
    XR_CHECK(xrCreateActionSet(instance, &info, &m_actionSet), "xrCreateActionSet");
    return true;
}

static XrAction createAction(XrActionSet set, const char* name, const char* localName,
                               XrActionType type, XrPath* subPaths, uint32_t subCount) {
    XrActionCreateInfo info = {XR_TYPE_ACTION_CREATE_INFO};
    strncpy(info.actionName, name, XR_MAX_ACTION_NAME_SIZE);
    strncpy(info.localizedActionName, localName, XR_MAX_LOCALIZED_ACTION_NAME_SIZE);
    info.actionType = type;
    info.subactionPaths = subPaths;
    info.countSubactionPaths = subCount;
    XrAction action;
    xrCreateAction(set, &info, &action);
    return action;
}

bool ControllerPoller::createActions() {
    m_poseAction = createAction(m_actionSet, "hand_pose", "Hand Pose",
        XR_ACTION_TYPE_POSE_INPUT, m_handPaths, 2);
    m_triggerAction = createAction(m_actionSet, "trigger", "Trigger",
        XR_ACTION_TYPE_FLOAT_INPUT, m_handPaths, 2);
    m_gripAction = createAction(m_actionSet, "grip", "Grip",
        XR_ACTION_TYPE_FLOAT_INPUT, m_handPaths, 2);
    m_thumbstickAction = createAction(m_actionSet, "thumbstick", "Thumbstick",
        XR_ACTION_TYPE_VECTOR2F_INPUT, m_handPaths, 2);
    m_aAction = createAction(m_actionSet, "a_button", "A/X Button",
        XR_ACTION_TYPE_BOOLEAN_INPUT, m_handPaths, 2);
    m_bAction = createAction(m_actionSet, "b_button", "B/Y Button",
        XR_ACTION_TYPE_BOOLEAN_INPUT, m_handPaths, 2);
    m_menuAction = createAction(m_actionSet, "menu", "Menu",
        XR_ACTION_TYPE_BOOLEAN_INPUT, m_handPaths, 2);
    return true;
}

bool ControllerPoller::suggestBindings(XrInstance instance) {
    // Use Khronos simple controller profile as default
    XrPath profilePath;
    xrStringToPath(instance, "/interaction_profiles/khr/simple_controller", &profilePath);

    auto pathOf = [&](const char* p) -> XrPath {
        XrPath path; xrStringToPath(instance, p, &path); return path;
    };

    XrActionSuggestedBinding bindings[] = {
        {m_poseAction, pathOf("/user/hand/left/input/grip/pose")},
        {m_poseAction, pathOf("/user/hand/right/input/grip/pose")},
        {m_triggerAction, pathOf("/user/hand/left/input/select/click")},
        {m_triggerAction, pathOf("/user/hand/right/input/select/click")},
        {m_menuAction, pathOf("/user/hand/left/input/menu/click")},
        {m_menuAction, pathOf("/user/hand/right/input/menu/click")},
    };

    XrInteractionProfileSuggestedBinding suggestion = {
        XR_TYPE_INTERACTION_PROFILE_SUGGESTED_BINDING};
    suggestion.interactionProfile = profilePath;
    suggestion.suggestedBindings = bindings;
    suggestion.countSuggestedBindings = sizeof(bindings) / sizeof(bindings[0]);

    XrResult result = xrSuggestInteractionProfileBindings(instance, &suggestion);
    if (XR_FAILED(result)) {
        LOGW("Suggest bindings failed (result=%d), controllers may not work", (int)result);
    }
    return true;
}

bool ControllerPoller::attachActionSet(XrSession session) {
    XrSessionActionSetsAttachInfo attachInfo = {XR_TYPE_SESSION_ACTION_SETS_ATTACH_INFO};
    attachInfo.actionSets = &m_actionSet;
    attachInfo.countActionSets = 1;
    XR_CHECK(xrAttachSessionActionSets(session, &attachInfo), "xrAttachSessionActionSets");
    return true;
}

void ControllerPoller::pollAndSend(XrSession session, XrSpace stageSpace,
                                    XrTime predictedTime, TrackingSender& sender) {
    if (!m_initialized) return;

    // Sync actions
    XrActiveActionSet activeSet = {m_actionSet, XR_NULL_PATH};
    XrActionsSyncInfo syncInfo = {XR_TYPE_ACTIONS_SYNC_INFO};
    syncInfo.activeActionSets = &activeSet;
    syncInfo.countActiveActionSets = 1;
    xrSyncActions(session, &syncInfo);

    for (int hand = 0; hand < 2; hand++) {
        // Get pose
        XrSpaceLocation loc = {XR_TYPE_SPACE_LOCATION};
        xrLocateSpace(m_handSpaces[hand], stageSpace, predictedTime, &loc);

        bool poseValid = (loc.locationFlags &
            (XR_SPACE_LOCATION_POSITION_VALID_BIT | XR_SPACE_LOCATION_ORIENTATION_VALID_BIT))
            == (XR_SPACE_LOCATION_POSITION_VALID_BIT | XR_SPACE_LOCATION_ORIENTATION_VALID_BIT);

        if (!poseValid) continue;

        // Get trigger
        XrActionStateFloat triggerState = {XR_TYPE_ACTION_STATE_FLOAT};
        XrActionStateGetInfo getInfo = {XR_TYPE_ACTION_STATE_GET_INFO};
        getInfo.action = m_triggerAction;
        getInfo.subactionPath = m_handPaths[hand];
        xrGetActionStateFloat(session, &getInfo, &triggerState);

        // Get grip
        XrActionStateFloat gripState = {XR_TYPE_ACTION_STATE_FLOAT};
        getInfo.action = m_gripAction;
        xrGetActionStateFloat(session, &getInfo, &gripState);

        // Get thumbstick
        XrActionStateVector2f thumbState = {XR_TYPE_ACTION_STATE_VECTOR2F};
        getInfo.action = m_thumbstickAction;
        xrGetActionStateVector2f(session, &getInfo, &thumbState);

        // Get buttons
        uint32_t flags = 0;
        auto getButton = [&](XrAction action, uint32_t bit) {
            XrActionStateBoolean state = {XR_TYPE_ACTION_STATE_BOOLEAN};
            getInfo.action = action;
            xrGetActionStateBoolean(session, &getInfo, &state);
            if (state.currentState) flags |= bit;
        };
        getButton(m_aAction, 0x01);
        getButton(m_bAction, 0x02);
        getButton(m_menuAction, 0x04);

        sender.sendControllerState(
            (uint8_t)hand, loc.pose, predictedTime,
            triggerState.currentState, gripState.currentState,
            thumbState.currentState.x, thumbState.currentState.y,
            flags, 100 // battery placeholder
        );
    }
}
