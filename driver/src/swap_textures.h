#pragma once

#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>
#include <string>
#include <vector>

/**
 * The swap texture sets SteamVR's compositor renders into
 * (IVRDriverDirectModeComponent::CreateSwapTextureSet and friends). The
 * compositor runs in its own process: it opens each texture from the DXGI
 * shared handle returned here, and hands the same handle back in
 * SubmitLayer. No OpenVR calls, so it is tested against a WARP device
 * (tests/test_d3d_pipeline.cpp).
 */
class SwapTextureSets {
public:
    static constexpr int kTexturesPerSet = 3;

    struct Texture {
        uint64_t handle = 0;  // DXGI shared handle, as SteamVR passes it around
        uint32_t pid = 0;
        uint32_t setId = 0;
        DXGI_FORMAT format = DXGI_FORMAT_UNKNOWN;
        Microsoft::WRL::ComPtr<ID3D11Texture2D> texture;
        Microsoft::WRL::ComPtr<ID3D11ShaderResourceView> view;  // for EyeBlit
    };

    /// Create a set of `kTexturesPerSet` shareable textures for process `pid`
    /// and write their shared handles to `outHandles`. On failure nothing is
    /// kept and `error` says why.
    bool create(ID3D11Device* device, uint32_t pid, uint32_t width, uint32_t height,
                DXGI_FORMAT format, uint32_t sampleCount, uint64_t (&outHandles)[kTexturesPerSet],
                std::string& error);

    /// The texture with shared handle `handle`, or nullptr.
    const Texture* find(uint64_t handle) const;

    /// Destroy the whole set `handle` belongs to (any one of its handles
    /// names the set).
    void destroySet(uint64_t handle);

    /// Destroy every set of process `pid`.
    void destroyAll(uint32_t pid);

    size_t size() const { return m_textures.size(); }

private:
    std::vector<Texture> m_textures;
    uint32_t m_nextSetId = 0;
};
