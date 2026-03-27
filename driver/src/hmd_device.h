#pragma once

#include <openvr_driver.h>
#include "direct_mode.h"
#include <atomic>

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

    uint32_t GetObjectId() const { return m_objectId; }

private:
    void SetupProperties();

    uint32_t m_objectId = vr::k_unTrackedDeviceIndexInvalid;
    vr::PropertyContainerHandle_t m_propertyContainer = vr::k_ulInvalidPropertyContainer;

    CDirectModeComponent m_directMode;

    // Current pose from the streaming engine
    std::atomic<bool> m_poseValid{false};
    vr::DriverPose_t m_pose{};
};
