#pragma once

#include <openvr_driver.h>
#include "direct_mode.h"
#include "display_component.h"
#include <atomic>
#include <thread>

/**
 * HMD tracked device. Represents the virtual HMD that SteamVR sees.
 * Receives tracking data from the Rust streaming engine (which gets it from the real HMD).
 */
class CHmdDevice : public vr::ITrackedDeviceServerDriver
{
public:
    CHmdDevice();
    ~CHmdDevice();

    // ITrackedDeviceServerDriver
    vr::EVRInitError Activate(uint32_t unObjectId) override;
    void Deactivate() override;
    void EnterStandby() override {}
    void* GetComponent(const char* pchComponentNameAndVersion) override;
    void DebugRequest(const char* pchRequest, char* pchResponseBuffer, uint32_t unResponseBufferSize) override;
    vr::DriverPose_t GetPose() override;

    void RunFrame();

    /// Forward IDR request to the DirectMode NVENC encoder.
    void requestIdr() { m_directMode.requestIdr(); }

    /// Forward gaze data for foveated encoding.
    void updateGaze(float x, float y, bool valid) { m_directMode.updateGaze(x, y, valid); }

    /// Forward a new target bitrate to the encoder.
    void updateBitrate(uint32_t bitrateBps) { m_directMode.updateBitrate(bitrateBps); }

    uint32_t GetObjectId() const { return m_objectId; }

private:
    void SetupProperties();
    void startVsync();
    void stopVsync();

    uint32_t m_objectId = vr::k_unTrackedDeviceIndexInvalid;
    vr::PropertyContainerHandle_t m_propertyContainer = vr::k_ulInvalidPropertyContainer;

    CDirectModeComponent m_directMode;
    CDisplayComponent m_display;
    float m_refreshRate = 90.0f;

    // Direct mode has no display to take vsync from, so the driver sends
    // SteamVR a vsync event every refresh interval
    // (Prop_DriverDirectModeSendsVsyncEvents_Bool).
    std::thread m_vsyncThread;
    std::atomic<bool> m_vsyncRunning{false};

    // Current pose from the streaming engine
    std::atomic<bool> m_poseValid{false};
    vr::DriverPose_t m_pose{};
};
