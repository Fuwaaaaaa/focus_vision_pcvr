#include "direct_mode.h"
extern "C" {
#include "streaming_engine.h"
}
#include <cstring>

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
    NvencEncoder::Config encConfig;
    encConfig.width = width;
    encConfig.height = height;
    encConfig.fps = 90;
    encConfig.bitrate_bps = 80'000'000;
    encConfig.use_hevc = true;

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
    if (!pOutSwapTextureSet)
        return;

    SwapTexture tex;
    tex.handle = m_nextHandle++;
    tex.pid = unPid;
    m_swapTextures.push_back(tex);

    // Return 3 textures in the set (triple buffering)
    for (uint32_t i = 0; i < 3; i++)
    {
        pOutSwapTextureSet->rSharedTextureHandles[i] = tex.handle + i;
    }
    m_nextHandle += 3;

    vr::VRDriverLog()->Log("Focus Vision PCVR: CreateSwapTextureSet\n");
}

void CDirectModeComponent::DestroySwapTextureSet(vr::SharedTextureHandle_t sharedTextureHandle)
{
    for (auto it = m_swapTextures.begin(); it != m_swapTextures.end(); ++it)
    {
        if (it->handle == sharedTextureHandle)
        {
            m_swapTextures.erase(it);
            break;
        }
    }
}

void CDirectModeComponent::DestroyAllSwapTextureSets(uint32_t unPid)
{
    auto it = m_swapTextures.begin();
    while (it != m_swapTextures.end())
    {
        if (it->pid == unPid)
            it = m_swapTextures.erase(it);
        else
            ++it;
    }
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
    // Store the left-eye texture handle for encoding in Present().
    // In production, we'd combine both eyes or encode them separately.
    // For v1.0, we encode the left eye texture as a single stream.
    //
    // The SharedTextureHandle_t from SteamVR maps to an ID3D11Texture2D
    // via the swap texture set. The actual D3D11 resource resolution
    // happens when initEncoder() provides the D3D11 device.
    //
    // Note: we don't resolve the handle to a D3D11 texture here because
    // SteamVR may still be writing to it. We wait until Present() which
    // signals the frame is complete.
    (void)perEye; // Used in production for texture handle extraction
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

    // Encode the frame.
    // In the full implementation, we would:
    //   1. Resolve syncTexture to an ID3D11Texture2D
    //   2. Call m_frameCopy.copyFrame() for safe handoff
    //   3. Pass copied texture to m_encoder.encode()
    //
    // For now, we pass nullptr which triggers the test pattern path
    // in NvencEncoder, generating synthetic NAL data for pipeline validation.
    std::vector<uint8_t> nalData;
    bool isIdr = false;

    if (!m_encoder.encode(nullptr, false, nalData, isIdr)) {
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
