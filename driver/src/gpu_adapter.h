#pragma once

#include <cstdint>
#include <string>
#include <vector>

/**
 * Which GPU the driver renders and encodes on. SteamVR's compositor renders
 * on the adapter the HMD names in Prop_GraphicsAdapterLuid_Uint64, and the
 * swap textures it renders into are created on that adapter by this driver,
 * so both sides must agree — and NVENC needs it to be an NVIDIA GPU.
 */
namespace fvp_gpu {

inline constexpr uint32_t kNvidiaVendorId = 0x10DE;

struct AdapterInfo {
    uint32_t vendorId = 0;
    uint64_t luid = 0;
    uint64_t dedicatedVideoMemory = 0;
    bool software = false;  // WARP / Microsoft Basic Render Driver
};

/// Index into `adapters` to use: the NVIDIA adapter with the most video
/// memory (NVENC); failing that, the hardware adapter with the most (the
/// headset still gets tracked and SteamVR still runs, it just cannot
/// encode); -1 if there are only software adapters.
inline int chooseAdapter(const std::vector<AdapterInfo>& adapters) {
    int best = -1;
    auto better = [&](const AdapterInfo& a, const AdapterInfo& b) {
        const bool aNvidia = a.vendorId == kNvidiaVendorId;
        const bool bNvidia = b.vendorId == kNvidiaVendorId;
        if (aNvidia != bNvidia) return aNvidia;
        return a.dedicatedVideoMemory > b.dedicatedVideoMemory;
    };
    for (int i = 0; i < static_cast<int>(adapters.size()); i++) {
        if (adapters[i].software) continue;
        if (best < 0 || better(adapters[i], adapters[best])) best = i;
    }
    return best;
}

/// A LUID as SteamVR takes it in Prop_GraphicsAdapterLuid_Uint64: the
/// 8 bytes of the LUID struct (LowPart, then HighPart).
inline uint64_t packLuid(uint32_t lowPart, int32_t highPart) {
    return (static_cast<uint64_t>(static_cast<uint32_t>(highPart)) << 32) | lowPart;
}

}  // namespace fvp_gpu

#ifdef _WIN32
#include <d3d11.h>
#include <wrl/client.h>

namespace fvp_gpu {

/// Create the D3D11 device on the adapter `chooseAdapter` picks. On success
/// `luid` is that adapter's and `description` names it (for the log);
/// otherwise `description` says why.
bool createDevice(Microsoft::WRL::ComPtr<ID3D11Device>& device, uint64_t& luid,
                  std::string& description);

}  // namespace fvp_gpu
#endif
