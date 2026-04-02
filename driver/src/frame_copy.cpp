#include "frame_copy.h"
#include <cstdio>

bool FrameCopy::init(ID3D11Device* device, uint32_t width, uint32_t height) {
    m_width = width;
    m_height = height;

    D3D11_TEXTURE2D_DESC desc = {};
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM; // Common format for VR compositors
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED; // Allows sharing with NVENC

    for (int i = 0; i < 2; i++) {
        HRESULT hr = device->CreateTexture2D(&desc, nullptr, &m_staging[i]);
        if (FAILED(hr)) {
            return false;
        }
    }

    m_initialized = true;
    return true;
}

void FrameCopy::shutdown() {
    m_staging[0].Reset();
    m_staging[1].Reset();
    m_initialized = false;
}

ID3D11Texture2D* FrameCopy::copyFrame(ID3D11DeviceContext* context, ID3D11Texture2D* source) {
    if (!m_initialized || !source) return nullptr;

    ID3D11Texture2D* dest = m_staging[m_currentBuffer].Get();
    context->CopyResource(dest, source);
    // Flush submits GPU commands but does NOT wait for completion.
    // This is safe because NVENC's nvEncMapInputResource uses the same
    // D3D11 device context, so GPU command ordering is guaranteed.
    // The copy will complete before NVENC reads the texture.
    context->Flush();

    m_currentBuffer = (m_currentBuffer + 1) % 2; // Flip double buffer
    return dest;
}
