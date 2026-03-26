#include "server_driver.h"
#include <cstring>

// The single server driver instance
static CServerDriver g_serverDriver;

/**
 * Main entry point called by SteamVR when loading the driver DLL.
 * Returns the requested interface, or nullptr if not supported.
 */
extern "C" __declspec(dllexport)
void* HmdDriverFactory(const char* pInterfaceName, int* pReturnCode)
{
    if (strcmp(pInterfaceName, vr::IServerTrackedDeviceProvider_Version) == 0)
    {
        return &g_serverDriver;
    }

    if (pReturnCode)
        *pReturnCode = vr::VRInitError_Init_InterfaceNotFound;

    return nullptr;
}
