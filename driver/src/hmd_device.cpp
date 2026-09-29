#include "hmd_device.h"
extern "C" {
#include "streaming_engine.h"
}
#include "driver_log.h"
#include <cstring>

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

    // Before the properties: they name the device's GPU.
    m_directMode.init();
    SetupProperties();

    driverLog("HMD Activated");
    return vr::VRInitError_None;
}

void CHmdDevice::Deactivate()
{
    driverLog("HMD Deactivated");
    m_objectId = vr::k_unTrackedDeviceIndexInvalid;
}

void* CHmdDevice::GetComponent(const char* pchComponentNameAndVersion)
{
    if (strcmp(pchComponentNameAndVersion, vr::IVRDriverDirectModeComponent_Version) == 0)
    {
        return static_cast<vr::IVRDriverDirectModeComponent*>(&m_directMode);
    }
    if (strcmp(pchComponentNameAndVersion, vr::IVRDisplayComponent_Version) == 0)
    {
        return static_cast<vr::IVRDisplayComponent*>(&m_display);
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

void CHmdDevice::updateViewConfig(const FvpViewConfig& view)
{
    std::lock_guard<std::mutex> lock(m_viewMutex);
    m_pendingView = view;
}

void CHmdDevice::applyViewConfig(const FvpViewConfig& view)
{
    const fvp_display::Fov left = fvp_display::fovFromRadians(
        view.left_eye[0], view.left_eye[1], view.left_eye[2], view.left_eye[3]);
    const fvp_display::Fov right = fvp_display::fovFromRadians(
        view.right_eye[0], view.right_eye[1], view.right_eye[2], view.right_eye[3]);
    m_display.setFov(left, right);

    auto rect = [](const fvp_display::Fov& fov) {
        const fvp_display::ProjectionRaw p = fvp_display::projectionRaw(fov);
        vr::HmdRect2_t r{};
        r.vTopLeft.v[0] = p.left;
        r.vTopLeft.v[1] = p.top;
        r.vBottomRight.v[0] = p.right;
        r.vBottomRight.v[1] = p.bottom;
        return r;
    };
    auto matrix = [](const fvp_display::EyeToHead& e) {
        vr::HmdMatrix34_t m{};
        for (int r = 0; r < 3; r++)
            for (int c = 0; c < 4; c++) m.m[r][c] = e.m[r][c];
        return m;
    };
    auto host = vr::VRServerDriverHost();
    host->SetDisplayProjectionRaw(m_objectId, rect(left), rect(right));
    host->SetDisplayEyeToHead(m_objectId, matrix(fvp_display::eyeToHead(-view.ipd_m / 2)),
                              matrix(fvp_display::eyeToHead(view.ipd_m / 2)));
    vr::VRProperties()->SetFloatProperty(m_propertyContainer, vr::Prop_UserIpdMeters_Float, view.ipd_m);

    driverLog("Headset view: left eye %.1f/%.1f/%.1f/%.1f, right eye %.1f/%.1f/%.1f/%.1f degrees "
              "(left/right/up/down), IPD %.1f mm",
              left.left, left.right, left.up, left.down, right.left, right.right, right.up, right.down,
              view.ipd_m * 1000.0f);
}

void CHmdDevice::RunFrame()
{
    // A new headset view (a session started, or the IPD dial moved)
    std::optional<FvpViewConfig> view;
    {
        std::lock_guard<std::mutex> lock(m_viewMutex);
        view.swap(m_pendingView);
    }
    if (view && m_objectId != vr::k_unTrackedDeviceIndexInvalid)
    {
        applyViewConfig(*view);
    }

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

    // Load display config from Rust streaming engine
    FvpConfig fvpConfig{};
    float ipd = 0.063f;
    float refreshRate = 90.0f;
    float vsyncToPhotons = 0.011f;
    uint32_t eyeWidth = 1832;
    uint32_t eyeHeight = 1920;
    if (fvp_get_config(&fvpConfig) == 0)
    {
        ipd = fvpConfig.ipd;
        refreshRate = fvpConfig.refresh_rate;
        vsyncToPhotons = fvpConfig.seconds_from_vsync_to_photons;
        eyeWidth = fvpConfig.render_width;
        eyeHeight = fvpConfig.render_height;
        driverLog("Config loaded from streaming engine");
    }
    else
    {
        driverLog("Using default display config");
    }
    m_display.configure(eyeWidth, eyeHeight, fvp_display::kDefaultFov);

    // Device identification
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ModelNumber_String, "Focus Vision PCVR");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_SerialNumber_String, "FVP_HMD_001");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_ManufacturerName_String, "FocusVisionPCVR");
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_TrackingSystemName_String, "focus_vision_pcvr");

    // Display properties — from shared config
    props->SetFloatProperty(m_propertyContainer,
        vr::Prop_UserIpdMeters_Float, ipd);
    props->SetFloatProperty(m_propertyContainer,
        vr::Prop_DisplayFrequency_Float, refreshRate);
    props->SetFloatProperty(m_propertyContainer,
        vr::Prop_SecondsFromVsyncToPhotons_Float, vsyncToPhotons);

    props->SetUint64Property(m_propertyContainer,
        vr::Prop_CurrentUniverseId_Uint64, 2);

    // Report as a VR HMD (not a controller or tracker)
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_IsOnDesktop_Bool, false);

    // Direct mode: the compositor renders on the driver's GPU (the swap
    // textures live there). SteamVR times vsync itself; PostPresent paces
    // the frames (FramePacer), as ALVR does.
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_HasDisplayComponent_Bool, true);
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_HasDriverDirectModeComponent_Bool, true);
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_DriverDirectModeSendsVsyncEvents_Bool, false);
    if (m_directMode.adapterLuid() != 0)
    {
        props->SetUint64Property(m_propertyContainer,
            vr::Prop_GraphicsAdapterLuid_Uint64, m_directMode.adapterLuid());
    }

    // Firmware version
    props->SetUint64Property(m_propertyContainer,
        vr::Prop_FirmwareVersion_Uint64, 1);
    props->SetStringProperty(m_propertyContainer,
        vr::Prop_RenderModelName_String, "generic_hmd");
}
