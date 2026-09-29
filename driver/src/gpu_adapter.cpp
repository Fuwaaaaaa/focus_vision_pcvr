#include "gpu_adapter.h"

#include <dxgi.h>
#include <cstdio>
#include <iterator>

using Microsoft::WRL::ComPtr;

namespace fvp_gpu {

bool createDevice(ComPtr<ID3D11Device>& device, uint64_t& luid, std::string& description) {
    ComPtr<IDXGIFactory1> factory;
    if (FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory)))) {
        description = "CreateDXGIFactory1 failed";
        return false;
    }

    std::vector<ComPtr<IDXGIAdapter1>> adapters;
    std::vector<AdapterInfo> infos;
    std::vector<std::wstring> names;
    ComPtr<IDXGIAdapter1> adapter;
    for (UINT i = 0; factory->EnumAdapters1(i, &adapter) != DXGI_ERROR_NOT_FOUND; i++) {
        DXGI_ADAPTER_DESC1 desc{};
        if (FAILED(adapter->GetDesc1(&desc))) continue;
        AdapterInfo info;
        info.vendorId = desc.VendorId;
        info.luid = packLuid(desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart);
        info.dedicatedVideoMemory = desc.DedicatedVideoMemory;
        info.software = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) != 0;
        adapters.push_back(adapter);
        infos.push_back(info);
        names.emplace_back(desc.Description);
    }

    const int chosen = chooseAdapter(infos);
    if (chosen < 0) {
        description = "no hardware GPU found";
        return false;
    }

    const D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0};
    // The adapter is given, so the driver type must be UNKNOWN.
    HRESULT hr = D3D11CreateDevice(adapters[chosen].Get(), D3D_DRIVER_TYPE_UNKNOWN, nullptr,
                                   D3D11_CREATE_DEVICE_BGRA_SUPPORT, levels,
                                   static_cast<UINT>(std::size(levels)), D3D11_SDK_VERSION,
                                   &device, nullptr, nullptr);
    char buf[320];
    if (FAILED(hr)) {
        snprintf(buf, sizeof(buf), "D3D11CreateDevice on %ls failed (hr=0x%08lx)",
                 names[chosen].c_str(), static_cast<unsigned long>(hr));
        description = buf;
        return false;
    }

    luid = infos[chosen].luid;
    snprintf(buf, sizeof(buf), "%ls (vendor 0x%04x, %llu MB)%s", names[chosen].c_str(),
             infos[chosen].vendorId,
             static_cast<unsigned long long>(infos[chosen].dedicatedVideoMemory >> 20),
             infos[chosen].vendorId == kNvidiaVendorId ? "" : " - not NVIDIA, NVENC unavailable");
    description = buf;
    return true;
}

}  // namespace fvp_gpu
