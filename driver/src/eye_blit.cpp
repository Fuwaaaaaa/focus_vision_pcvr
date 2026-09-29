#include "eye_blit.h"

#include <d3dcompiler.h>
#include <cstdio>
#include <cstring>

using Microsoft::WRL::ComPtr;

namespace {

// A triangle covering the output; each pixel samples the matching point of
// the source region. uvRect is (u0, v0, u1, v1).
const char kShader[] = R"(
cbuffer Params : register(b0) {
    float4 uvRect;
    uint encodeSrgb;
    uint3 padding;
};
Texture2D source : register(t0);
SamplerState linearClamp : register(s0);

struct VsOut {
    float4 pos : SV_Position;
    float2 uv : TEXCOORD0;
};

VsOut vs_main(uint id : SV_VertexID) {
    float2 t = float2((id << 1) & 2, id & 2);  // (0,0) (2,0) (0,2)
    VsOut o;
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1);
    o.uv = lerp(uvRect.xy, uvRect.zw, t);
    return o;
}

float3 linearToSrgb(float3 c) {
    c = saturate(c);
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

float4 ps_main(VsOut i) : SV_Target {
    float3 c = source.Sample(linearClamp, i.uv).rgb;
    if (encodeSrgb) c = linearToSrgb(c);
    return float4(c, 1);
}
)";

struct Params {
    float uvRect[4];
    uint32_t encodeSrgb;
    uint32_t padding[3];
};
static_assert(sizeof(Params) % 16 == 0, "constant buffers are 16-byte multiples");

bool compile(const char* entry, const char* target, ComPtr<ID3DBlob>& code, std::string& error) {
    ComPtr<ID3DBlob> messages;
    HRESULT hr = D3DCompile(kShader, sizeof(kShader) - 1, "eye_blit", nullptr, nullptr, entry,
                            target, D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &code, &messages);
    if (FAILED(hr)) {
        error = std::string("shader ") + entry + ": " +
                (messages ? static_cast<const char*>(messages->GetBufferPointer()) : "compile failed");
        return false;
    }
    return true;
}

}  // namespace

bool EyeBlit::init(ID3D11Device* device, uint32_t width, uint32_t height, std::string& error) {
    shutdown();

    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = width;
    desc.Height = height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = kOutputFormat;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET;
    char buf[128];
    HRESULT hr = device->CreateTexture2D(&desc, nullptr, &m_output);
    if (SUCCEEDED(hr)) hr = device->CreateRenderTargetView(m_output.Get(), nullptr, &m_outputView);
    if (FAILED(hr)) {
        snprintf(buf, sizeof(buf), "encoder input texture %ux%u (hr=0x%08lx)", width, height,
                 static_cast<unsigned long>(hr));
        error = buf;
        shutdown();
        return false;
    }

    ComPtr<ID3DBlob> vs, ps;
    if (!compile("vs_main", "vs_4_0", vs, error) || !compile("ps_main", "ps_4_0", ps, error)) {
        shutdown();
        return false;
    }
    hr = device->CreateVertexShader(vs->GetBufferPointer(), vs->GetBufferSize(), nullptr, &m_vertexShader);
    if (SUCCEEDED(hr)) {
        hr = device->CreatePixelShader(ps->GetBufferPointer(), ps->GetBufferSize(), nullptr, &m_pixelShader);
    }

    D3D11_BUFFER_DESC params{};
    params.ByteWidth = sizeof(Params);
    params.Usage = D3D11_USAGE_DEFAULT;
    params.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
    if (SUCCEEDED(hr)) hr = device->CreateBuffer(&params, nullptr, &m_params);

    D3D11_SAMPLER_DESC sampler{};
    sampler.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
    sampler.AddressU = sampler.AddressV = sampler.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
    sampler.MaxLOD = D3D11_FLOAT32_MAX;
    if (SUCCEEDED(hr)) hr = device->CreateSamplerState(&sampler, &m_sampler);

    if (FAILED(hr)) {
        snprintf(buf, sizeof(buf), "blit pipeline (hr=0x%08lx)", static_cast<unsigned long>(hr));
        error = buf;
        shutdown();
        return false;
    }
    m_width = width;
    m_height = height;
    return true;
}

void EyeBlit::shutdown() {
    m_sampler.Reset();
    m_params.Reset();
    m_pixelShader.Reset();
    m_vertexShader.Reset();
    m_outputView.Reset();
    m_output.Reset();
    m_width = m_height = 0;
}

bool EyeBlit::draw(ID3D11DeviceContext* context, ID3D11ShaderResourceView* source, DXGI_FORMAT format,
                   const fvp_blit::UvRect& uv) {
    if (!m_output || !source) return false;

    Params params{};
    params.uvRect[0] = uv.u0;
    params.uvRect[1] = uv.v0;
    params.uvRect[2] = uv.u1;
    params.uvRect[3] = uv.v1;
    params.encodeSrgb = fvp_blit::needsSrgbEncode(format) ? 1u : 0u;
    context->UpdateSubresource(m_params.Get(), 0, nullptr, &params, 0, 0);

    D3D11_VIEWPORT viewport{};
    viewport.Width = static_cast<float>(m_width);
    viewport.Height = static_cast<float>(m_height);
    viewport.MaxDepth = 1.0f;

    // The device is the driver's own; nothing else sets state on it, so
    // the draw sets what it needs and restores nothing.
    context->OMSetRenderTargets(1, m_outputView.GetAddressOf(), nullptr);
    context->OMSetBlendState(nullptr, nullptr, 0xffffffff);
    context->RSSetState(nullptr);
    context->RSSetViewports(1, &viewport);
    context->IASetInputLayout(nullptr);
    context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    context->VSSetShader(m_vertexShader.Get(), nullptr, 0);
    context->VSSetConstantBuffers(0, 1, m_params.GetAddressOf());
    context->PSSetShader(m_pixelShader.Get(), nullptr, 0);
    context->PSSetConstantBuffers(0, 1, m_params.GetAddressOf());
    context->PSSetShaderResources(0, 1, &source);
    context->PSSetSamplers(0, 1, m_sampler.GetAddressOf());
    context->Draw(3, 0);

    // Unbind the source: the next frame's swap texture is a render target
    // on the compositor's side.
    ID3D11ShaderResourceView* none = nullptr;
    context->PSSetShaderResources(0, 1, &none);
    return true;
}
