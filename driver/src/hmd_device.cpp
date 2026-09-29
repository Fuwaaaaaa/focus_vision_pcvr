#include "hmd_device.h"
extern "C" {
#include "streaming_engine.h"
}
#include "driver_log.h"
#include <chrono>
#include <cstring>
#include <windows.h>

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
    stopVsync();
}

vr::EVRInitError CHmdDevice::Activate(uint32_t unObjectId)
{
    m_objectId = unObjectId;
    m_propertyContainer = vr::VRProperties()->TrackedDeviceToPropertyContainer(unObjectId);

    // Before the properties: they name the device's GPU.
    m_directMode.init();
    SetupProperties();
    startVsync();

    driverLog("HMD Activated");
    return vr::VRInitError_None;
}

void CHmdDevice::Deactivate()
{
    stopVsync();
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

void CHmdDevice::startVsync()
{
    if (m_vsyncRunning.exchange(true))
        return;
    const auto period = std::chrono::duration_cast<std::chrono::steady_clock::duration>(
        std::chrono::duration<double>(1.0 / (m_refreshRate > 0.0f ? m_refreshRate : 90.0f)));
    m_vsyncThread = std::thread([this, period] {
        // A high-resolution waitable timer: the default sleep granularity
        // (~15.6 ms) is longer than a 90 Hz frame.
        HANDLE timer = CreateWaitableTimerExW(nullptr, nullptr, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                                              TIMER_ALL_ACCESS);
        auto next = std::chrono::steady_clock::now() + period;
        while (m_vsyncRunning.load()) {
            const auto wait = next - std::chrono::steady_clock::now();
            if (wait > std::chrono::steady_clock::duration::zero()) {
                const auto ticks100ns = std::chrono::duration_cast<std::chrono::nanoseconds>(wait).count() / 100;
                LARGE_INTEGER due;
                due.QuadPart = -static_cast<LONGLONG>(ticks100ns);  // negative: relative
                if (timer && SetWaitableTimer(timer, &due, 0, nullptr, nullptr, FALSE)) {
                    WaitForSingleObject(timer, INFINITE);
                } else {
                    std::this_thread::sleep_until(next);
                }
            }
            vr::VRServerDriverHost()->VsyncEvent(0.0);
            next += period;
            // After a stall (debugger, suspended process), restart from now
            // rather than firing the missed events back to back.
            if (std::chrono::steady_clock::now() > next + period) {
                next = std::chrono::steady_clock::now() + period;
            }
        }
        if (timer) CloseHandle(timer);
    });
}

void CHmdDevice::stopVsync()
{
    if (!m_vsyncRunning.exchange(false))
        return;
    if (m_vsyncThread.joinable())
        m_vsyncThread.join();
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
    m_refreshRate = refreshRate;
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
    // textures live there) and takes vsync from the driver's events.
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_HasDisplayComponent_Bool, true);
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_HasDriverDirectModeComponent_Bool, true);
    props->SetBoolProperty(m_propertyContainer,
        vr::Prop_DriverDirectModeSendsVsyncEvents_Bool, true);
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
