#pragma once

#include <openvr_driver.h>
#include <vector>
#include <cstdint>

/**
 * Direct Mode component — captures frames from SteamVR's compositor.
 *
 * SteamVR calls Present() each frame with the rendered textures.
 * In v1.0, we perform a synchronous GPU→GPU copy of the texture
 * then hand it to the Rust streaming engine for encoding.
 *
 * Step 2: Stub implementation (no actual encoding).
 * Step 5: Will integrate with NVENC via Rust FFI.
 */
class CDirectModeComponent : public vr::IVRDriverDirectModeComponent
{
public:
    CDirectModeComponent();
    ~CDirectModeComponent();

    // IVRDriverDirectModeComponent
    void CreateSwapTextureSet(
        uint32_t unPid,
        const SwapTextureSetDesc_t* pSwapTextureSetDesc,
        SwapTextureSet_t* pOutSwapTextureSet) override;

    void DestroySwapTextureSet(vr::SharedTextureHandle_t sharedTextureHandle) override;
    void DestroyAllSwapTextureSets(uint32_t unPid) override;
    void GetNextSwapTextureSetIndex(
        vr::SharedTextureHandle_t sharedTextureHandles[2],
        uint32_t (*pIndices)[2]) override;

    void SubmitLayer(const SubmitLayerPerEye_t (&perEye)[2]) override;
    void Present(vr::SharedTextureHandle_t syncTexture) override;

    void PostPresent() override;
    void GetFrameTiming(vr::DriverDirectMode_FrameTiming* pFrameTiming) override;

private:
    uint32_t m_frameIndex = 0;

    // Swap texture set tracking
    struct SwapTexture {
        vr::SharedTextureHandle_t handle;
        uint32_t pid;
    };
    std::vector<SwapTexture> m_swapTextures;
    vr::SharedTextureHandle_t m_nextHandle = 1;
};
