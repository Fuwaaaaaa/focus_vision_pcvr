#pragma once

#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>

using Microsoft::WRL::ComPtr;

/**
 * Handles synchronous GPU→GPU copy of D3D11 textures for safe handoff
 * to the Rust streaming engine.
 *
 * SteamVR may reuse the source texture after Present() returns,
 * so we must copy it immediately (eng review decision: sync copy).
 * Double-buffered to avoid blocking Present() while previous copy
 * is being consumed by the encoder.
 */
class FrameCopy {
public:
    bool init(ID3D11Device* device, uint32_t width, uint32_t height);
    void shutdown();

    /// Copy source texture to our staging buffer. Returns the staging texture.
    /// Call Flush() after to ensure the copy completes before Present() returns.
    ID3D11Texture2D* copyFrame(ID3D11DeviceContext* context, ID3D11Texture2D* source);

private:
    ComPtr<ID3D11Texture2D> m_staging[2]; // Double buffer
    uint32_t m_currentBuffer = 0;
    uint32_t m_width = 0;
    uint32_t m_height = 0;
    bool m_initialized = false;
};
