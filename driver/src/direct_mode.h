#pragma once

#include <openvr_driver.h>
#include <d3d11.h>
#include <wrl/client.h>
#include <array>
#include <cstdint>
#include <mutex>
#include <vector>
#include "display_geometry.h"
#include "eye_blit.h"
#include "frame_pacer.h"
#include "layer_compose.h"
#include "nvenc_encoder.h"
#include "swap_textures.h"
#include "sync_texture.h"

/**
 * Direct mode: SteamVR's compositor (its own process) renders each frame's
 * layers into swap textures this driver creates on its D3D11 device, and
 * Present hands the frame over:
 *
 *   CreateSwapTextureSet -> shareable textures, DXGI shared handles
 *   SubmitLayer          -> collect the frame's layers: the scene first,
 *                           then overlays and the dashboard
 *   Present              -> with the sync texture held: EyeBlit (both
 *                           eyes side by side, the layers above the scene
 *                           blended on -> encoder input)
 *                           -> NvencEncoder -> fvp_submit_encoded_frame()
 *                           (with the scene's head orientation and the
 *                           time from the frame being ready to here)
 *   PostPresent          -> wait out the frame's slot (FramePacer)
 *
 * NVENC encoding runs in C++; only NAL byte arrays cross the C ABI into
 * Rust.
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

    /// New target bitrate, applied before the next encode. Thread-safe.
    void updateBitrate(uint32_t bitrateBps) { m_encoder.requestBitrate(bitrateBps); }

    /// Each eye's field of view SteamVR renders with (the display
    /// component's), for turning layers to the scene's pose. Thread-safe.
    void setFov(const fvp_display::Fov& left, const fvp_display::Fov& right);

    /// Layers kept per frame; more are dropped (and logged).
    static constexpr size_t kMaxLayers = 16;

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

    /// Waits out the rest of the frame's slot at the refresh rate
    /// (FramePacer): SteamVR times vsync itself and renders the next frame
    /// when this returns.
    void PostPresent(const Throttling_t* pThrottling) override;

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
    FramePacer m_pacer;
    void* m_pacingTimer = nullptr;  // high-resolution waitable timer (HANDLE)

    // The layers submitted since the last Present, the scene first.
    // SubmitLayer and Present come on the same thread.
    std::vector<std::array<SubmitLayerPerEye_t, 2>> m_layers;

    std::mutex m_fovMutex;
    fvp_layers::Tangents m_eyeFov[2] = {fvp_layers::tangents(fvp_display::kDefaultFov),
                                        fvp_layers::tangents(fvp_display::kDefaultFov)};

    uint32_t m_frameIndex = 0;
    std::vector<uint8_t> m_nal;
    uint32_t m_droppedLayers = 0;
    uint32_t m_unreadableOverlays = 0;
    uint32_t m_unknownTextures = 0;
    uint32_t m_syncTimeouts = 0;
    uint32_t m_encodeFailures = 0;
    uint32_t m_submitFailures = 0;
};
