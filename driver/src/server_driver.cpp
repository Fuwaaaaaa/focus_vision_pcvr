#include "server_driver.h"
extern "C" {
#include "streaming_engine.h"
}
#include <cstdio>

static vr::IVRDriverContext* s_pDriverContext = nullptr;

// Global pointer for IDR callback to reach the encoder.
// Set during Init(), cleared during Cleanup().
static CServerDriver* s_instance = nullptr;

static void onIdrRequest() {
    // Called from Rust TCP control thread when HMD sends IDR_REQUEST
    if (s_instance) {
        s_instance->requestIdr();
    }
}

vr::EVRInitError CServerDriver::Init(vr::IVRDriverContext* pDriverContext)
{
    s_pDriverContext = pDriverContext;
    vr::EVRInitError err = vr::InitServerDriverContext(pDriverContext);
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

    // Create and register controllers
    m_leftController = std::make_unique<CControllerDevice>(true);
    vr::VRServerDriverHost()->TrackedDeviceAdded(
        m_leftController->GetSerialNumber(),
        vr::TrackedDeviceClass_Controller,
        m_leftController.get()
    );

    m_rightController = std::make_unique<CControllerDevice>(false);
    vr::VRServerDriverHost()->TrackedDeviceAdded(
        m_rightController->GetSerialNumber(),
        vr::TrackedDeviceClass_Controller,
        m_rightController.get()
    );

    vr::VRDriverLog()->Log("Focus Vision PCVR: Controllers added\n");

    // Register IDR callback so TCP IDR_REQUEST reaches the NVENC encoder
    s_instance = this;
    fvp_set_idr_callback(onIdrRequest);

    return vr::VRInitError_None;
}

void CServerDriver::Cleanup()
{
    vr::VRDriverLog()->Log("Focus Vision PCVR: Driver Cleanup\n");

    m_leftController.reset();
    m_rightController.reset();
    m_hmdDevice.reset();

    // SAFETY: fvp_shutdown() must be called BEFORE clearing s_instance.
    // fvp_shutdown() cancels the Tokio runtime, which stops the TCP control
    // reader task. That task is the only caller of the IDR callback (onIdrRequest),
    // which accesses s_instance. By shutting down Tokio first, we guarantee
    // no callback can fire after s_instance is nulled.
    fvp_shutdown();
    s_instance = nullptr;

    vr::CleanupDriverContext();
}

const char* const* CServerDriver::GetInterfaceVersions()
{
    return vr::k_InterfaceVersions;
}

void CServerDriver::requestIdr()
{
    if (m_hmdDevice) {
        // Forward to DirectMode's NVENC encoder via HMD device
        // HMD's DirectMode component handles the actual encoder
        m_hmdDevice->requestIdr();
    }
}

void CServerDriver::RunFrame()
{
    if (m_hmdDevice) m_hmdDevice->RunFrame();
    if (m_leftController) m_leftController->RunFrame();
    if (m_rightController) m_rightController->RunFrame();
}
