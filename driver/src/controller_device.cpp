#include "controller_device.h"
extern "C" {
#include "streaming_engine.h"
}
#include <cstring>
#include <cstdio>

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
    return m_isLeft ? "FVP_CTRL_LEFT" : "FVP_CTRL_RIGHT";
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
    return m_pose;
}

void CControllerDevice::SetupProperties()
{
    auto props = vr::VRProperties();

    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ModelNumber_String, "Focus Vision PCVR Controller");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_SerialNumber_String, GetSerialNumber());
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ManufacturerName_String, "FocusVisionPCVR");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_TrackingSystemName_String, "focus_vision_pcvr");

    props->SetInt32Property(m_propertyContainer,
        vr::Prop_ControllerRoleHint_Int32,
        m_isLeft ? vr::TrackedControllerRole_LeftHand : vr::TrackedControllerRole_RightHand);

    // Input profile (SteamVR uses this to map controls)
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_InputProfilePath_String, "{focus_vision_pcvr}/input/controller_profile.json");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_RenderModelName_String, "generic_controller");
}

void CControllerDevice::CreateInputComponents()
{
    auto input = vr::VRDriverInput();

    // Buttons (boolean)
    input->CreateBooleanComponent(m_propertyContainer, "/input/a/click", &m_hA);
    input->CreateBooleanComponent(m_propertyContainer, "/input/b/click", &m_hB);
    input->CreateBooleanComponent(m_propertyContainer, "/input/system/click", &m_hSystem);
    input->CreateBooleanComponent(m_propertyContainer, "/input/application_menu/click", &m_hMenu);
    input->CreateBooleanComponent(m_propertyContainer, "/input/joystick/click", &m_hThumbstickClick);

    // Analog axes (scalar)
    input->CreateScalarComponent(m_propertyContainer, "/input/trigger/value",
        &m_hTrigger, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedOneSided);
    input->CreateScalarComponent(m_propertyContainer, "/input/grip/value",
        &m_hGrip, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedOneSided);
    input->CreateScalarComponent(m_propertyContainer, "/input/joystick/x",
        &m_hJoystickX, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedTwoSided);
    input->CreateScalarComponent(m_propertyContainer, "/input/joystick/y",
        &m_hJoystickY, vr::VRScalarType_Absolute, vr::VRScalarUnits_NormalizedTwoSided);

    // Touch sensors (boolean)
    input->CreateBooleanComponent(m_propertyContainer, "/input/trigger/touch", &m_hTriggerTouch);
    input->CreateBooleanComponent(m_propertyContainer, "/input/joystick/touch", &m_hThumbstickTouch);
    input->CreateBooleanComponent(m_propertyContainer, "/input/grip/touch", &m_hGripTouch);

    // Haptic output
    input->CreateHapticComponent(m_propertyContainer, "/output/haptic", &m_hHaptic);
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

    if (result == 0)
    {
        // Update pose
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

        // Update input components
        auto input = vr::VRDriverInput();
        input->UpdateScalarComponent(m_hTrigger, state.trigger, 0.0);
        input->UpdateScalarComponent(m_hGrip, state.grip, 0.0);
        input->UpdateScalarComponent(m_hJoystickX, state.thumbstick_x, 0.0);
        input->UpdateScalarComponent(m_hJoystickY, state.thumbstick_y, 0.0);

        input->UpdateBooleanComponent(m_hA,
            (state.button_flags & 0x01) != 0, 0.0);  // A_X_PRESSED
        input->UpdateBooleanComponent(m_hB,
            (state.button_flags & 0x02) != 0, 0.0);  // B_Y_PRESSED
        input->UpdateBooleanComponent(m_hMenu,
            (state.button_flags & 0x04) != 0, 0.0);  // MENU_PRESSED
        input->UpdateBooleanComponent(m_hSystem,
            (state.button_flags & 0x08) != 0, 0.0);  // SYSTEM_PRESSED
        input->UpdateBooleanComponent(m_hThumbstickClick,
            (state.button_flags & 0x10) != 0, 0.0);  // THUMBSTICK_CLICK
        input->UpdateBooleanComponent(m_hTriggerTouch,
            (state.button_flags & 0x20) != 0, 0.0);  // TRIGGER_TOUCH
        input->UpdateBooleanComponent(m_hThumbstickTouch,
            (state.button_flags & 0x40) != 0, 0.0);  // THUMBSTICK_TOUCH
        input->UpdateBooleanComponent(m_hGripTouch,
            (state.button_flags & 0x80) != 0, 0.0);  // GRIP_TOUCH
    }
    else
    {
        m_pose.poseIsValid = false;
        m_pose.result = vr::TrackingResult_Calibrating_InProgress;
    }

    // Push pose to SteamVR
    if (m_objectId != vr::k_unTrackedDeviceIndexInvalid)
    {
        vr::VRServerDriverHost()->TrackedDevicePoseUpdated(
            m_objectId, m_pose, sizeof(m_pose));
    }
}
