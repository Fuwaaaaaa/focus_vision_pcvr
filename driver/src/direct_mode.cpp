#include "direct_mode.h"
extern "C" {
#include "streaming_engine.h"
}
#include <cstring>
#include <algorithm>

CDirectModeComponent::CDirectModeComponent()
{
}

CDirectModeComponent::~CDirectModeComponent()
{
    m_encoder.shutdown();
    m_frameCopy.shutdown();
}

bool CDirectModeComponent::initEncoder(ID3D11Device* device, uint32_t width, uint32_t height)
{
    // Read config from Rust streaming engine (single source of truth)
    FvpConfig fvpCfg = {};
    NvencEncoder::Config encConfig;
    if (fvp_get_config(&fvpCfg) == 0) {
        encConfig.width = fvpCfg.render_width;
        encConfig.height = fvpCfg.render_height;
        encConfig.fps = (uint32_t)fvpCfg.refresh_rate;
        encConfig.bitrate_bps = fvpCfg.render_width * fvpCfg.render_height * 2; // ~80Mbps at native res
        encConfig.use_hevc = true;
    } else {
        // Fallback if engine not initialized yet
        encConfig.width = width;
        encConfig.height = height;
        encConfig.fps = 90;
        encConfig.bitrate_bps = 80'000'000;
        encConfig.use_hevc = true;
    }

    if (!m_frameCopy.init(device, width, height)) {
        vr::VRDriverLog()->Log("Focus Vision PCVR: FrameCopy init failed\n");
        return false;
    }

    if (!m_encoder.init(device, encConfig)) {
        vr::VRDriverLog()->Log("Focus Vision PCVR: NvencEncoder init failed\n");
        return false;
    }

    m_encoderReady = true;
    vr::VRDriverLog()->Log("Focus Vision PCVR: Encoder initialized\n");
    return true;
}

void CDirectModeComponent::requestIdr()
{
    m_encoder.requestIdr();
}

void CDirectModeComponent::CreateSwapTextureSet(
    uint32_t unPid,
    const SwapTextureSetDesc_t* pSwapTextureSetDesc,
    SwapTextureSet_t* pOutSwapTextureSet)
{
    if (!pOutSwapTextureSet || !pSwapTextureSetDesc)
        return;

    // Create real D3D11 textures that SteamVR's compositor will render into.
    // Triple-buffered: 3 textures per swap set, rotated by GetNextSwapTextureSetIndex.
    D3D11_TEXTURE2D_DESC desc = {};
    desc.Width = pSwapTextureSetDesc->nWidth;
    desc.Height = pSwapTextureSetDesc->nHeight;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = static_cast<DXGI_FORMAT>(pSwapTextureSetDesc->nFormat);
    desc.SampleDesc.Count = pSwapTextureSetDesc->nSampleCount > 0 ? pSwapTextureSetDesc->nSampleCount : 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
    desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED;

    ID3D11Device* device = m_encoder.getDevice();
    if (!device) {
        vr::VRDriverLog()->Log("Focus Vision PCVR: CreateSwapTextureSet — no D3D11 device\n");
        return;
    }

    for (uint32_t i = 0; i < 3; i++)
    {
        vr::SharedTextureHandle_t handle = m_nextHandle++;

        SwapTextureEntry entry;
        entry.handle = handle;
        entry.pid = unPid;

        HRESULT hr = device->CreateTexture2D(&desc, nullptr, &entry.texture);
        if (FAILED(hr)) {
            char buf[128];
            snprintf(buf, sizeof(buf),
                "Focus Vision PCVR: CreateTexture2D failed for swap set (hr=0x%08lx)\n", hr);
            vr::VRDriverLog()->Log(buf);
            return;
        }

        pOutSwapTextureSet->rSharedTextureHandles[i] = handle;
        m_swapTextures.push_back(std::move(entry));
    }

    char buf[128];
    snprintf(buf, sizeof(buf),
        "Focus Vision PCVR: CreateSwapTextureSet %ux%u (3 textures)\n",
        pSwapTextureSetDesc->nWidth, pSwapTextureSetDesc->nHeight);
    vr::VRDriverLog()->Log(buf);
}

void CDirectModeComponent::DestroySwapTextureSet(vr::SharedTextureHandle_t sharedTextureHandle)
{
    // Clear m_pendingTexture if it points to a texture being destroyed,
    // to prevent use-after-free if NVENC hasn't consumed it yet.
    for (const auto& entry : m_swapTextures) {
        if (entry.handle == sharedTextureHandle && entry.texture.Get() == m_pendingTexture) {
            m_pendingTexture = nullptr;
        }
    }
    m_swapTextures.erase(
        std::remove_if(m_swapTextures.begin(), m_swapTextures.end(),
            [sharedTextureHandle](const SwapTextureEntry& e) { return e.handle == sharedTextureHandle; }),
        m_swapTextures.end());
}

void CDirectModeComponent::DestroyAllSwapTextureSets(uint32_t unPid)
{
    // Clear m_pendingTexture if it belongs to the destroyed PID
    for (const auto& entry : m_swapTextures) {
        if (entry.pid == unPid && entry.texture.Get() == m_pendingTexture) {
            m_pendingTexture = nullptr;
        }
    }
    m_swapTextures.erase(
        std::remove_if(m_swapTextures.begin(), m_swapTextures.end(),
            [unPid](const SwapTextureEntry& e) { return e.pid == unPid; }),
        m_swapTextures.end());
}

void CDirectModeComponent::GetNextSwapTextureSetIndex(
    vr::SharedTextureHandle_t sharedTextureHandles[2],
    uint32_t (*pIndices)[2])
{
    // Simple round-robin through 3 textures
    static uint32_t s_index = 0;
    (*pIndices)[0] = s_index;
    (*pIndices)[1] = s_index;
    s_index = (s_index + 1) % 3;
}

void CDirectModeComponent::SubmitLayer(const SubmitLayerPerEye_t (&perEye)[2])
{
    // Resolve left-eye SharedTextureHandle_t to ID3D11Texture2D via our map.
    // The handle was created by CreateSwapTextureSet and maps to a real D3D11 texture.
    // For v1.0, left eye only (single stream). Right eye ignored.
    vr::SharedTextureHandle_t leftHandle = perEye[0].hTexture;
    if (leftHandle == INVALID_SHARED_TEXTURE_HANDLE)
        return;

    for (const auto& entry : m_swapTextures) {
        if (entry.handle == leftHandle) {
            m_pendingTexture = entry.texture.Get();
            return;
        }
    }
}

void CDirectModeComponent::Present(vr::SharedTextureHandle_t syncTexture)
{
    m_frameIndex++;

    if (!m_encoderReady) {
        // Encoder not yet initialized — log periodically as before
        if (m_frameIndex % 900 == 0) {
            char buf[128];
            snprintf(buf, sizeof(buf),
                "Focus Vision PCVR: Present() frame %u (encoder not ready)\n", m_frameIndex);
            vr::VRDriverLog()->Log(buf);
        }
        return;
    }

    // Encode the frame: copy pending texture, then encode.
    // m_pendingTexture is set by SubmitLayer() with the left-eye D3D11 resource.
    // FrameCopy does a sync GPU→GPU copy (double-buffered) so SteamVR can
    // safely reuse the source texture after Present() returns.
    // If no texture was submitted (e.g. first frame), fall through to
    // NvencEncoder's test pattern path.
    ID3D11Texture2D* encodeInput = nullptr;

    if (m_pendingTexture) {
        ComPtr<ID3D11DeviceContext> ctx;
        m_encoder.getDevice()->GetImmediateContext(&ctx);
        encodeInput = m_frameCopy.copyFrame(ctx.Get(), m_pendingTexture);
        m_pendingTexture = nullptr; // Consumed
    }

    std::vector<uint8_t> nalData;
    bool isIdr = false;

    if (!m_encoder.encode(encodeInput, false, nalData, isIdr)) {
        return;
    }

    // Submit encoded NAL data to Rust streaming engine for RTP packetization
    int32_t result = fvp_submit_encoded_nal(
        nalData.data(),
        static_cast<uint32_t>(nalData.size()),
        m_frameIndex,
        isIdr ? 1 : 0
    );

    if (result != 0 && m_frameIndex % 900 == 0) {
        vr::VRDriverLog()->Log("Focus Vision PCVR: fvp_submit_encoded_nal failed\n");
    }
}

void CDirectModeComponent::GetFrameTiming(vr::DriverDirectMode_FrameTiming* pFrameTiming)
{
    if (pFrameTiming)
    {
        pFrameTiming->m_nSize = sizeof(vr::DriverDirectMode_FrameTiming);
        pFrameTiming->m_nNumFramePresents = 1;
        pFrameTiming->m_nNumMisPresented = 0;
        pFrameTiming->m_nNumDroppedFrames = 0;
        pFrameTiming->m_nReprojectionFlags = 0;
    }
}
