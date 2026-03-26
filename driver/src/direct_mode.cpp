#include "direct_mode.h"
#include "streaming_engine.h"
#include <cstring>

CDirectModeComponent::CDirectModeComponent()
{
}

CDirectModeComponent::~CDirectModeComponent()
{
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
    // Step 5 will capture texture handles here
    // For now, just acknowledge the layer submission
}

void CDirectModeComponent::Present(vr::SharedTextureHandle_t syncTexture)
{
    m_frameIndex++;

    // Step 2: Stub — just log periodically
    if (m_frameIndex % 900 == 0) // ~every 10 seconds at 90fps
    {
        char buf[128];
        snprintf(buf, sizeof(buf),
            "Focus Vision PCVR: Present() frame %u\n", m_frameIndex);
        vr::VRDriverLog()->Log(buf);
    }

    // Step 5 will:
    // 1. Get the D3D11 texture from syncTexture
    // 2. CopyResource() to our staging texture (synchronous GPU copy)
    // 3. Call fvp_submit_frame() to pass to Rust for encoding
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
