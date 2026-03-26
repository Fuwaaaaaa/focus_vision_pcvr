#include "hmd_device.h"
#include "streaming_engine.h"
#include <cstring>

// Display properties — VIVE Focus Vision native resolution
static constexpr uint32_t kRenderWidth = 1832;
static constexpr uint32_t kRenderHeight = 1920;
static constexpr float kRefreshRate = 90.0f;
static constexpr float kIPD = 0.063f; // meters

CHmdDevice::CHmdDevice()
{
    memset(&m_pose, 0, sizeof(m_pose));
    m_pose.poseIsValid = false;
    m_pose.result = vr::TrackingResult_Uninitialized;
    m_pose.deviceIsConnected = true;

    // Identity rotation
    m_pose.qWorldFromDriverRotation.w = 1.0;
    m_pose.qDriverFromHeadRotation.w = 1.0;
    m_pose.qRotation.w = 1.0;
}

CHmdDevice::~CHmdDevice()
{
}

vr::EVRInitError CHmdDevice::Activate(uint32_t unObjectId)
{
    m_objectId = unObjectId;
    m_propertyContainer = vr::VRProperties()->TrackedDeviceToPropertyContainer(unObjectId);

    SetupProperties();

    vr::VRDriverLog()->Log("Focus Vision PCVR: HMD Activated\n");
    return vr::VRInitError_None;
}

void CHmdDevice::Deactivate()
{
    vr::VRDriverLog()->Log("Focus Vision PCVR: HMD Deactivated\n");
    m_objectId = vr::k_unTrackedDeviceIndexInvalid;
}

void* CHmdDevice::GetComponent(const char* pchComponentNameAndVersion)
{
    if (strcmp(pchComponentNameAndVersion, vr::IVRDriverDirectModeComponent_Version) == 0)
    {
        return static_cast<vr::IVRDriverDirectModeComponent*>(&m_directMode);
    }

    return nullptr;
}

void CHmdDevice::DebugRequest(const char* /*pchRequest*/, char* pchResponseBuffer, uint32_t unResponseBufferSize)
{
    if (unResponseBufferSize > 0)
        pchResponseBuffer[0] = '\0';
}

vr::DriverPose_t CHmdDevice::GetPose()
{
    return m_pose;
}

void CHmdDevice::RunFrame()
{
    // Try to get tracking data from the Rust streaming engine
    TrackingData trackingData;
    int32_t result = fvp_get_tracking_data(&trackingData);

    if (result == 0)
    {
        // Valid tracking data received
        m_pose.poseIsValid = true;
        m_pose.result = vr::TrackingResult_Running_OK;
        m_pose.deviceIsConnected = true;

        // Position
        m_pose.vecPosition[0] = trackingData.position[0];
        m_pose.vecPosition[1] = trackingData.position[1];
        m_pose.vecPosition[2] = trackingData.position[2];

        // Orientation (quaternion)
        m_pose.qRotation.x = trackingData.orientation[0];
        m_pose.qRotation.y = trackingData.orientation[1];
        m_pose.qRotation.z = trackingData.orientation[2];
        m_pose.qRotation.w = trackingData.orientation[3];

        m_poseValid.store(true);
    }
    else
    {
        // No tracking data yet — report as calibrating
        m_pose.poseIsValid = false;
        m_pose.result = vr::TrackingResult_Calibrating_InProgress;
        m_pose.deviceIsConnected = true;
    }

    // Push the updated pose to SteamVR
    if (m_objectId != vr::k_unTrackedDeviceIndexInvalid)
    {
        vr::VRServerDriverHost()->TrackedDevicePoseUpdated(
            m_objectId, m_pose, sizeof(m_pose));
    }
}

void CHmdDevice::SetupProperties()
{
    auto props = vr::VRProperties();

    // Device identification
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ModelNumber_String, "Focus Vision PCVR");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_SerialNumber_String, "FVP_HMD_001");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ManufacturerName_String, "FocusVisionPCVR");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_TrackingSystemName_String, "focus_vision_pcvr");

    // Display properties
    props->SetFloatProperty(m_propertyContainer,
        vr::Prop_UserIpdMeters_Float, kIPD);
    props->SetFloatProperty(m_propertyContainer,
        vr::Prop_DisplayFrequency_Float, kRefreshRate);
    props->SetFloatProperty(m_propertyContainer,
        vr::Prop_SecondsFromVsyncToPhotons_Float, 0.011f);

    props->SetUint64Property(m_propertyContainer,
        vr::Prop_CurrentUniverseId_Uint64, 2);

    // Report as a VR HMD (not a controller or tracker)
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_IsOnDesktop_Bool, false);

    // Firmware version
    props->SetUint64Property(m_propertyContainer,
        vr::Prop_FirmwareVersion_Uint64, 1);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_RenderModelName_String, "generic_hmd");
}
