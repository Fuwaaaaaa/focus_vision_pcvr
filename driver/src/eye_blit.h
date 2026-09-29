#pragma once

#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>
#include <string>

namespace fvp_blit {

/// Whether sampling `format` yields linear light, which has to be encoded
/// back to sRGB for the video: sRGB formats (the sampler decodes them) and
/// float formats (linear HDR).
inline bool needsSrgbEncode(DXGI_FORMAT format) {
    switch (format) {
    case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
    case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB:
    case DXGI_FORMAT_B8G8R8X8_UNORM_SRGB:
    case DXGI_FORMAT_R16G16B16A16_FLOAT:
    case DXGI_FORMAT_R11G11B10_FLOAT:
    case DXGI_FORMAT_R32G32B32A32_FLOAT:
        return true;
    default:
        return false;
    }
}

/// A region of a texture in UV (0..1). The layer's VRTextureBounds_t maps
/// straight onto it; u0 > u1 or v0 > v1 means the image is flipped.
struct UvRect {
    float u0 = 0.0f;
    float v0 = 0.0f;
    float u1 = 1.0f;
    float v1 = 1.0f;
};

}  // namespace fvp_blit

/**
 * Draws the eyes of a submitted layer into the encoder's input texture,
 * side by side (left eye in the left half): each eye's region of its swap
 * texture, scaled to the encoded size, in the format NVENC reads
 * (B8G8R8A8, NV_ENC_BUFFER_FORMAT_ARGB). A draw rather than CopyResource,
 * which D3D11 silently skips between format groups (the compositor's
 * R8G8B8A8 vs NVENC's B8G8R8A8) and which can't scale (SteamVR
 * supersampling changes the eye size) or pick one eye out of a double-wide
 * texture.
 */
class EyeBlit {
public:
    static constexpr DXGI_FORMAT kOutputFormat = DXGI_FORMAT_B8G8R8A8_UNORM;

    /// Create the output texture — two eyes of `eyeWidth` × `eyeHeight`
    /// side by side — and the shaders.
    bool init(ID3D11Device* device, uint32_t eyeWidth, uint32_t eyeHeight, std::string& error);
    void shutdown();

    /// The texture draw() fills; NVENC registers it as its input.
    ID3D11Texture2D* output() const { return m_output.Get(); }

    /// Draw the `uv` region of the texture behind `source` (of `format`) to
    /// fill eye `eye`'s half of output() (0 = left, 1 = right). Queued on
    /// `context`; false if not initialized.
    bool draw(ID3D11DeviceContext* context, ID3D11ShaderResourceView* source, DXGI_FORMAT format,
              const fvp_blit::UvRect& uv, int eye);

private:
    Microsoft::WRL::ComPtr<ID3D11Texture2D> m_output;
    Microsoft::WRL::ComPtr<ID3D11RenderTargetView> m_outputView;
    Microsoft::WRL::ComPtr<ID3D11VertexShader> m_vertexShader;
    Microsoft::WRL::ComPtr<ID3D11PixelShader> m_pixelShader;
    Microsoft::WRL::ComPtr<ID3D11Buffer> m_params;
    Microsoft::WRL::ComPtr<ID3D11SamplerState> m_sampler;
    uint32_t m_eyeWidth = 0;
    uint32_t m_eyeHeight = 0;
};
