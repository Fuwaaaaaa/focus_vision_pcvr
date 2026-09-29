#include "eye_blit.h"

#include <d3dcompiler.h>
#include <cstdio>
#include <cstring>

using Microsoft::WRL::ComPtr;

namespace {

// A triangle covering the output. Each pixel is a point of the eye image
// (t: 0..1, y down) and samples the matching point of the source region
// (uvRect: u0, v0, u1, v1). A layer above the scene is blended by its alpha
// and, if it was rendered at another head orientation, turned to the
// scene's: fvp_layers::layerPoint (layer_compose.h), which the tests check.
const char kShader[] = R"(
cbuffer Params : register(b0) {
    float4 uvRect;
    uint encodeSrgb;
    uint over;          // a layer above the scene: blended, keeps its alpha
    uint rotated;
    uint padding;
    float4 eyeTangents; // left, right, up, down
    float4 rotation[3]; // rows (xyz): the scene's head frame -> the layer's
};
Texture2D source : register(t0);
SamplerState linearClamp : register(s0);

struct VsOut {
    float4 pos : SV_Position;
    float2 t : TEXCOORD0;
};

VsOut vs_main(uint id : SV_VertexID) {
    float2 t = float2((id << 1) & 2, id & 2);  // (0,0) (2,0) (0,2)
    VsOut o;
    o.pos = float4(t * float2(2, -2) + float2(-1, 1), 0, 1);
    o.t = t;
    return o;
}

float3 linearToSrgb(float3 c) {
    c = saturate(c);
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

float4 ps_main(VsOut i) : SV_Target {
    float2 p = i.t;
    if (rotated) {
        float3 d = float3(lerp(eyeTangents.x, eyeTangents.y, p.x), lerp(eyeTangents.z, eyeTangents.w, p.y), -1);
        float3 r = float3(dot(rotation[0].xyz, d), dot(rotation[1].xyz, d), dot(rotation[2].xyz, d));
        if (r.z > -1e-4) discard;
        float2 q = r.xy / -r.z;
        p = float2((q.x - eyeTangents.x) / (eyeTangents.y - eyeTangents.x),
                   (eyeTangents.z - q.y) / (eyeTangents.z - eyeTangents.w));
        if (any(p < 0) || any(p > 1)) discard;  // outside the layer's image
    }
    float4 c = source.Sample(linearClamp, lerp(uvRect.xy, uvRect.zw, p));
    if (encodeSrgb) c.rgb = linearToSrgb(c.rgb);
    // The scene is opaque: some apps submit it with zero alpha.
    return float4(c.rgb, over ? c.a : 1);
}
)";

struct Params {
    float uvRect[4];
    uint32_t encodeSrgb;
    uint32_t over;
    uint32_t rotated;
    uint32_t padding;
    float eyeTangents[4];
    float rotation[3][4];
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

bool EyeBlit::init(ID3D11Device* device, uint32_t eyeWidth, uint32_t eyeHeight, std::string& error) {
    shutdown();

    const uint32_t width = eyeWidth * 2;  // both eyes side by side
    const uint32_t height = eyeHeight;
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

    // Layers above the scene: source over by alpha (straight, as ALVR).
    D3D11_BLEND_DESC blend{};
    D3D11_RENDER_TARGET_BLEND_DESC& target = blend.RenderTarget[0];
    target.BlendEnable = TRUE;
    target.SrcBlend = D3D11_BLEND_SRC_ALPHA;
    target.DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
    target.BlendOp = D3D11_BLEND_OP_ADD;
    target.SrcBlendAlpha = D3D11_BLEND_ONE;
    target.DestBlendAlpha = D3D11_BLEND_ZERO;
    target.BlendOpAlpha = D3D11_BLEND_OP_ADD;
    target.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
    if (SUCCEEDED(hr)) hr = device->CreateBlendState(&blend, &m_blendOver);

    if (FAILED(hr)) {
        snprintf(buf, sizeof(buf), "blit pipeline (hr=0x%08lx)", static_cast<unsigned long>(hr));
        error = buf;
        shutdown();
        return false;
    }
    m_eyeWidth = eyeWidth;
    m_eyeHeight = eyeHeight;
    return true;
}

void EyeBlit::shutdown() {
    m_blendOver.Reset();
    m_sampler.Reset();
    m_params.Reset();
    m_pixelShader.Reset();
    m_vertexShader.Reset();
    m_outputView.Reset();
    m_output.Reset();
    m_eyeWidth = m_eyeHeight = 0;
}

bool EyeBlit::draw(ID3D11DeviceContext* context, ID3D11ShaderResourceView* source, DXGI_FORMAT format,
                   const fvp_blit::UvRect& uv, int eye) {
    return drawLayer(context, source, format, uv, eye, false, fvp_layers::Placement{}, fvp_layers::Tangents{});
}

bool EyeBlit::drawOver(ID3D11DeviceContext* context, ID3D11ShaderResourceView* source, DXGI_FORMAT format,
                       const fvp_blit::UvRect& uv, int eye, const fvp_layers::Placement& placement,
                       const fvp_layers::Tangents& eyeFov) {
    return drawLayer(context, source, format, uv, eye, true, placement, eyeFov);
}

bool EyeBlit::drawLayer(ID3D11DeviceContext* context, ID3D11ShaderResourceView* source, DXGI_FORMAT format,
                        const fvp_blit::UvRect& uv, int eye, bool over, const fvp_layers::Placement& placement,
                        const fvp_layers::Tangents& eyeFov) {
    if (!m_output || !source || eye < 0 || eye > 1) return false;

    Params params{};
    params.uvRect[0] = uv.u0;
    params.uvRect[1] = uv.v0;
    params.uvRect[2] = uv.u1;
    params.uvRect[3] = uv.v1;
    params.encodeSrgb = fvp_blit::needsSrgbEncode(format) ? 1u : 0u;
    params.over = over ? 1u : 0u;
    params.rotated = over && placement.rotated ? 1u : 0u;
    params.eyeTangents[0] = eyeFov.left;
    params.eyeTangents[1] = eyeFov.right;
    params.eyeTangents[2] = eyeFov.up;
    params.eyeTangents[3] = eyeFov.down;
    for (int r = 0; r < 3; r++)
        for (int c = 0; c < 3; c++) params.rotation[r][c] = placement.rotation.m[r][c];
    context->UpdateSubresource(m_params.Get(), 0, nullptr, &params, 0, 0);

    D3D11_VIEWPORT viewport{};  // this eye's half of the output
    viewport.TopLeftX = static_cast<float>(eye * m_eyeWidth);
    viewport.Width = static_cast<float>(m_eyeWidth);
    viewport.Height = static_cast<float>(m_eyeHeight);
    viewport.MaxDepth = 1.0f;

    // The device is the driver's own; nothing else sets state on it, so
    // the draw sets what it needs and restores nothing.
    context->OMSetRenderTargets(1, m_outputView.GetAddressOf(), nullptr);
    context->OMSetBlendState(over ? m_blendOver.Get() : nullptr, nullptr, 0xffffffff);
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
