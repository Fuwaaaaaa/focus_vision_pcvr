#pragma once

#include <openvr_driver.h>
#include <memory>
#include "hmd_device.h"
#include "controller_device.h"

/**
 * Server-side driver provider. SteamVR uses this to discover and manage devices.
 */
class CServerDriver : public vr::IServerTrackedDeviceProvider
{
public:
    // IServerTrackedDeviceProvider
    vr::EVRInitError Init(vr::IVRDriverContext* pDriverContext) override;
    void Cleanup() override;
    const char* const* GetInterfaceVersions() override;
    void RunFrame() override;
    bool ShouldBlockStandbyMode() override { return false; }
    void EnterStandby() override {}
    void LeaveStandby() override {}

    /// Forward IDR request from TCP control to the NVENC encoder.
    void requestIdr();

private:
    std::unique_ptr<CHmdDevice> m_hmdDevice;
    std::unique_ptr<CControllerDevice> m_leftController;
    std::unique_ptr<CControllerDevice> m_rightController;
};
