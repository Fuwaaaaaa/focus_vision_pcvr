#pragma once

#include <openvr_driver.h>
#include <vector>
#include <cstdint>
#include "nvenc_encoder.h"
#include "frame_copy.h"

/**
 * Direct Mode component — captures frames from SteamVR's compositor.
 *
 * SubmitLayer() receives per-eye textures from SteamVR.
 * Present() triggers encoding and submission to the Rust streaming engine:
 *
 *   SubmitLayer() -> store texture handles
 *   Present()     -> FrameCopy -> NvencEncoder -> fvp_submit_encoded_nal()
 *
 * Architecture (eng review #1): NVENC encoding runs in C++.
 * Only NAL byte arrays cross the C ABI into Rust for RTP packetization.
 */
class CDirectModeComponent : public vr::IVRDriverDirectModeComponent
{
public:
    CDirectModeComponent();
    ~CDirectModeComponent();

    /// Initialize D3D11 device, frame copy, and NVENC encoder.
    bool initEncoder(ID3D11Device* device, uint32_t width, uint32_t height);

    /// Request an IDR keyframe on the next encode. Thread-safe.
    void requestIdr();

    /// Update gaze for foveated encoding. Thread-safe.
    void updateGaze(float x, float y, bool valid) { m_encoder.setGaze(x, y, valid); }

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

    void GetFrameTiming(vr::DriverDirectMode_FrameTiming* pFrameTiming) override;

private:
    uint32_t m_frameIndex = 0;

    // Video encoding pipeline
    NvencEncoder m_encoder;
    FrameCopy m_frameCopy;
    bool m_encoderReady = false;

    // Latest submitted texture (from SubmitLayer, consumed by Present).
    // ComPtr ensures the texture stays alive even if its swap set is destroyed
    // between SubmitLayer and Present.
    ComPtr<ID3D11Texture2D> m_pendingTexture;

    // Swap texture set: maps SharedTextureHandle_t → D3D11 texture.
    // CreateSwapTextureSet allocates real D3D11 textures and returns handles.
    // SubmitLayer resolves handles back to textures via this map.
    struct SwapTextureEntry {
        vr::SharedTextureHandle_t handle;
        uint32_t pid;
        uint32_t setId;      // Which swap set this belongs to
        uint32_t indexInSet;  // 0, 1, or 2 within the set
        ComPtr<ID3D11Texture2D> texture;
    };
    std::vector<SwapTextureEntry> m_swapTextures;
    vr::SharedTextureHandle_t m_nextHandle = 1;

    // Per swap-set round-robin index (setId → current index)
    std::vector<std::pair<uint32_t, uint32_t>> m_swapSetIndices; // (setId, currentIdx)
    uint32_t m_nextSetId = 0;
};
