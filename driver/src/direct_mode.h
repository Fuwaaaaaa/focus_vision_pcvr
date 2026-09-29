#pragma once

#include <openvr_driver.h>
#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>
#include <mutex>
#include <vector>
#include "eye_blit.h"
#include "nvenc_encoder.h"
#include "swap_textures.h"
#include "sync_texture.h"

/**
 * Direct mode: SteamVR's compositor (its own process) renders each frame's
 * layers into swap textures this driver creates on its D3D11 device, and
 * Present hands the frame over:
 *
 *   CreateSwapTextureSet -> shareable textures, DXGI shared handles
 *   SubmitLayer          -> remember the frame's first layer (the scene)
 *   Present              -> with the sync texture held: EyeBlit (left eye
 *                           -> encoder input) -> NvencEncoder
 *                           -> fvp_submit_encoded_nal()
 *
 * NVENC encoding runs in C++; only NAL byte arrays cross the C ABI into
 * Rust. Not yet: the right eye (stereo) and compositing the layers above
 * the scene (overlays, the SteamVR dashboard) — see TODOS.md.
 */
class CDirectModeComponent : public vr::IVRDriverDirectModeComponent
{
public:
    ~CDirectModeComponent();

    /// Create the D3D11 device and the encode path, from the engine's
    /// config. False without a usable GPU — SteamVR then has nothing to
    /// render on. An encoder that can't start is logged and leaves the
    /// device: SteamVR runs, nothing is streamed.
    bool init();

    /// The device's adapter, for Prop_GraphicsAdapterLuid_Uint64 (0 before
    /// init or without a GPU).
    uint64_t adapterLuid() const { return m_adapterLuid; }

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
    /// Log `what` on its 1st, 2nd, 4th, 8th... occurrence (per-frame
    /// problems that would otherwise fill the log).
    static void logSometimes(uint32_t& count, const char* what);

    Microsoft::WRL::ComPtr<ID3D11Device> m_device;
    Microsoft::WRL::ComPtr<ID3D11DeviceContext> m_context;
    uint64_t m_adapterLuid = 0;

    // SteamVR may create and destroy swap sets on another thread than the
    // one calling Present.
    std::mutex m_swapMutex;
    SwapTextureSets m_swapSets;

    EyeBlit m_eyeBlit;
    SyncTexture m_sync;
    NvencEncoder m_encoder;
    bool m_encoderReady = false;

    // The first layer submitted since the last Present: the scene.
    bool m_haveLayer = false;
    vr::SharedTextureHandle_t m_layerTexture = 0;
    vr::VRTextureBounds_t m_layerBounds{};

    uint32_t m_frameIndex = 0;
    std::vector<uint8_t> m_nal;
    uint32_t m_unknownTextures = 0;
    uint32_t m_syncTimeouts = 0;
    uint32_t m_encodeFailures = 0;
    uint32_t m_submitFailures = 0;
};
