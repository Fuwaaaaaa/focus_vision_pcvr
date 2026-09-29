#include "controller_device.h"
#include "touch_profile.h"
#include <cstring>
#include <cstdio>
#include <string>

CControllerDevice::CControllerDevice(bool isLeft)
    : m_isLeft(isLeft)
{
    memset(&m_pose, 0, sizeof(m_pose));
    m_pose.poseIsValid = false;
    m_pose.result = vr::TrackingResult_Uninitialized;
    m_pose.deviceIsConnected = true;
    m_pose.qWorldFromDriverRotation.w = 1.0;
    m_pose.qDriverFromHeadRotation.w = 1.0;
    m_pose.qRotation.w = 1.0;
}

const char* CControllerDevice::GetSerialNumber() const
{
    return fvp_touch::identity(m_isLeft).serial;
}

vr::EVRInitError CControllerDevice::Activate(uint32_t unObjectId)
{
    m_objectId = unObjectId;
    m_propertyContainer = vr::VRProperties()->TrackedDeviceToPropertyContainer(unObjectId);

    SetupProperties();
    CreateInputComponents();

    char buf[64];
    snprintf(buf, sizeof(buf), "Focus Vision PCVR: %s controller activated\n",
        m_isLeft ? "Left" : "Right");
    vr::VRDriverLog()->Log(buf);

    return vr::VRInitError_None;
}

void CControllerDevice::Deactivate()
{
    m_objectId = vr::k_unTrackedDeviceIndexInvalid;
}

void CControllerDevice::DebugRequest(const char*, char* pchResponseBuffer, uint32_t unResponseBufferSize)
{
    if (unResponseBufferSize > 0)
        pchResponseBuffer[0] = '\0';
}

vr::DriverPose_t CControllerDevice::GetPose()
{
    std::lock_guard<std::mutex> lock(m_poseMutex);
    return m_pose;
}

void CControllerDevice::SetupProperties()
{
    auto props = vr::VRProperties();
    const fvp_touch::Identity id = fvp_touch::identity(m_isLeft);

    // Oculus Touch (Quest 2) — see touch_profile.h for why.
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_TrackingSystemName_String, fvp_touch::kTrackingSystemName);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ManufacturerName_String, fvp_touch::kManufacturer);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ModelNumber_String, id.modelNumber);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_SerialNumber_String, id.serial);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_AttachedDeviceId_String, id.serial);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_RegisteredDeviceType_String, id.registeredDeviceType);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_RenderModelName_String, id.renderModel);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ControllerType_String, fvp_touch::kControllerType);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_InputProfilePath_String, fvp_touch::kInputProfilePath);

    const std::string icons = id.iconBase;
    const struct { vr::ETrackedDeviceProperty prop; const char* suffix; } iconFiles[] = {
        {vr::Prop_NamedIconPathDeviceOff_String, "_off.png"},
        {vr::Prop_NamedIconPathDeviceSearching_String, "_searching.gif"},
        {vr::Prop_NamedIconPathDeviceSearchingAlert_String, "_searching_alert.gif"},
        {vr::Prop_NamedIconPathDeviceReady_String, "_ready.png"},
        {vr::Prop_NamedIconPathDeviceReadyAlert_String, "_ready_alert.png"},
        {vr::Prop_NamedIconPathDeviceAlertLow_String, "_ready_low.png"},
        {vr::Prop_NamedIconPathDeviceStandby_String, "_standby.png"},
        {vr::Prop_NamedIconPathDeviceStandbyAlert_String, "_standby_alert.gif"},
    };
    for (const auto& icon : iconFiles) {
        props->SetStringProperty(m_propertyContainer, icon.prop, (icons + icon.suffix).c_str());
    }

    props->SetInt32Property(m_propertyContainer,
        vr::Prop_ControllerRoleHint_Int32,
        m_isLeft ? vr::TrackedControllerRole_LeftHand : vr::TrackedControllerRole_RightHand);
    props->SetInt32Property(m_propertyContainer,
        vr::Prop_Axis0Type_Int32, vr::k_eControllerAxis_Joystick);
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_DeviceProvidesBatteryStatus_Bool, false);
}

void CControllerDevice::CreateInputComponents()
{
    auto input = vr::VRDriverInput();
    const fvp_touch::FaceButtons face = fvp_touch::faceButtons(m_isLeft);

    // Buttons (boolean): Touch's paths — X / Y on the left, A / B on the right
    input->CreateBooleanComponent(m_propertyContainer, face.primaryClick, &m_hPrimary);
    input->CreateBooleanComponent(m_propertyContainer, face.secondaryClick, &m_hSecondary);
    input->CreateBooleanComponent(m_propertyContainer, fvp_touch::kSystemClick, &m_hSystem);
    input->CreateBooleanComponent(m_propertyContainer, fvp_touch::kStickClick, &m_hThumbstickClick);

    // Analog axes (scalar)
    input->CreateScalarComponent(m_propertyContainer, fvp_touch::kTriggerValue,
        &m_hTrigger, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedOneSided);
    input->CreateScalarComponent(m_propertyContainer, fvp_touch::kGripValue,
        &m_hGrip, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedOneSided);
    input->CreateScalarComponent(m_propertyContainer, fvp_touch::kStickX,
        &m_hJoystickX, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedTwoSided);
    input->CreateScalarComponent(m_propertyContainer, fvp_touch::kStickY,
        &m_hJoystickY, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedTwoSided);

    // Touch sensors (boolean)
    input->CreateBooleanComponent(m_propertyContainer, fvp_touch::kTriggerTouch, &m_hTriggerTouch);
    input->CreateBooleanComponent(m_propertyContainer, fvp_touch::kStickTouch, &m_hThumbstickTouch);
    input->CreateBooleanComponent(m_propertyContainer, fvp_touch::kGripTouch, &m_hGripTouch);

    // Haptic output
    input->CreateHapticComponent(m_propertyContainer, fvp_touch::kHaptic, &m_hHaptic);
}

void CControllerDevice::UpdateInputs(const ControllerState& state)
{
    auto input = vr::VRDriverInput();
    input->UpdateScalarComponent(m_hTrigger, state.trigger, 0.0);
    input->UpdateScalarComponent(m_hGrip, state.grip, 0.0);
    input->UpdateScalarComponent(m_hJoystickX, state.thumbstick_x, 0.0);
    input->UpdateScalarComponent(m_hJoystickY, state.thumbstick_y, 0.0);

    const fvp_touch::Buttons b = fvp_touch::buttons(state.button_flags);
    input->UpdateBooleanComponent(m_hPrimary, b.primary, 0.0);
    input->UpdateBooleanComponent(m_hSecondary, b.secondary, 0.0);
    input->UpdateBooleanComponent(m_hSystem, b.system, 0.0);
    input->UpdateBooleanComponent(m_hThumbstickClick, b.stickClick, 0.0);
    input->UpdateBooleanComponent(m_hTriggerTouch, b.triggerTouch, 0.0);
    input->UpdateBooleanComponent(m_hThumbstickTouch, b.stickTouch, 0.0);
    input->UpdateBooleanComponent(m_hGripTouch, b.gripTouch, 0.0);
}

void CControllerDevice::TriggerHaptic(float duration_s, float frequency, float amplitude)
{
    uint16_t duration_ms = static_cast<uint16_t>(duration_s * 1000.0f);
    if (duration_ms == 0) duration_ms = 1;
    fvp_haptic_event(m_isLeft ? 0 : 1, duration_ms, frequency, amplitude);
}

void CControllerDevice::RunFrame()
{
    uint8_t controllerId = m_isLeft ? 0 : 1;
    ControllerState state;

    int32_t result = fvp_get_controller_state(controllerId, &state);

    vr::DriverPose_t pose;
    {
        // SteamVR calls GetPose from its own threads.
        // REGRESSION: it read m_pose while this wrote it (a torn pose).
        std::lock_guard<std::mutex> lock(m_poseMutex);
        if (result == 0)
        {
            m_pose.poseIsValid = true;
            m_pose.result = vr::TrackingResult_Running_OK;
            m_pose.deviceIsConnected = true;

            m_pose.vecPosition[0] = state.position[0];
            m_pose.vecPosition[1] = state.position[1];
            m_pose.vecPosition[2] = state.position[2];

            m_pose.qRotation.x = state.orientation[0];
            m_pose.qRotation.y = state.orientation[1];
            m_pose.qRotation.z = state.orientation[2];
            m_pose.qRotation.w = state.orientation[3];
        }
        else
        {
            m_pose.poseIsValid = false;
            m_pose.result = vr::TrackingResult_Calibrating_InProgress;
        }
        pose = m_pose;
    }

    if (result == 0)
    {
        UpdateInputs(state);
        m_inputsLive = true;
    }
    else
    {
        // The controller stopped reporting (untracked, or the session
        // ended). SteamVR keeps the last value of every input, so release
        // them once — otherwise a held trigger or pushed stick stays that
        // way until the controller comes back.
        if (m_inputsLive)
        {
            UpdateInputs(ControllerState{});
            m_inputsLive = false;
        }
    }

    // Push pose to SteamVR
    if (m_objectId != vr::k_unTrackedDeviceIndexInvalid)
    {
        vr::VRServerDriverHost()->TrackedDevicePoseUpdated(
            m_objectId, pose, sizeof(pose));
    }
}
