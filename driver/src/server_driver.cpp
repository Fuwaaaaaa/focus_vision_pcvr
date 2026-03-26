#include "server_driver.h"
#include "streaming_engine.h"
#include <cstdio>

// Required macro to initialize the driver context
VR_INIT_DRIVER_CONTEXT(CServerDriver)

vr::EVRInitError CServerDriver::Init(vr::IVRDriverContext* pDriverContext)
{
    // Initialize driver context (sets up VRDriverLog, VRServerDriverHost, etc.)
    vr::EVRInitError err = InitServerDriverContext(pDriverContext);
    if (err != vr::VRInitError_None)
        return err;

    vr::VRDriverLog()->Log("Focus Vision PCVR: Driver Init\n");

    // Initialize the Rust streaming engine
    int32_t result = fvp_init();
    if (result != 0)
    {
        vr::VRDriverLog()->Log("Focus Vision PCVR: Failed to init streaming engine\n");
        return vr::VRInitError_Driver_Failed;
    }

    // Create and register the HMD device
    m_hmdDevice = std::make_unique<CHmdDevice>();
    vr::VRServerDriverHost()->TrackedDeviceAdded(
        "FVP_HMD_001",
        vr::TrackedDeviceClass_HMD,
        m_hmdDevice.get()
    );

    vr::VRDriverLog()->Log("Focus Vision PCVR: HMD device added\n");

    return vr::VRInitError_None;
}

void CServerDriver::Cleanup()
{
    vr::VRDriverLog()->Log("Focus Vision PCVR: Driver Cleanup\n");

    m_hmdDevice.reset();

    // Shut down the Rust streaming engine
    fvp_shutdown();

    CleanupDriverContext();
}

const char* const* CServerDriver::GetInterfaceVersions()
{
    return vr::k_InterfaceVersions;
}

void CServerDriver::RunFrame()
{
    if (m_hmdDevice)
    {
        m_hmdDevice->RunFrame();
    }
}
