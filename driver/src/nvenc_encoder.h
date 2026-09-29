#pragma once

#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>
#include <vector>
#include <atomic>

// Official NVENC API header (third_party/nvenc, SDK 12.2). The runtime,
// nvEncodeAPI64.dll, comes with the NVIDIA driver and is loaded with
// LoadLibrary, so building needs no SDK install.
#include "nvEncodeAPI.h"
#include "nvenc_config.h"

using Microsoft::WRL::ComPtr;

// ============================================================
// NvencEncoder class
// ============================================================

class NvencEncoder {
public:
    /// Where the encoder's messages go (SteamVR's vrserver.txt in the
    /// driver). Called with one line, without a trailing newline.
    using LogFn = void (*)(const char* message);

    struct Config {
        uint32_t width = 1832;
        uint32_t height = 1920;
        uint32_t fps = 90;
        uint32_t bitrate_bps = 80'000'000;
        bool use_hevc = true;
        bool full_range = true;  // Full RGB (0-255) vs limited (16-235)
        // Foveated encoding parameters (from Rust config via FFI)
        float fovea_radius = 0.15f;
        float mid_radius = 0.35f;
        int32_t mid_qp_offset = 5;
        int32_t peripheral_qp_offset = 15;
    };

    NvencEncoder() = default;
    ~NvencEncoder();

    NvencEncoder(const NvencEncoder&) = delete;
    NvencEncoder& operator=(const NvencEncoder&) = delete;

    /// Open an NVENC session on `device` that encodes `input` (B8G8R8A8,
    /// config.width × config.height; EyeBlit's output). False — with the
    /// reason logged — when NVENC is unavailable: not an NVIDIA GPU, an old
    /// driver, or no free session.
    bool init(ID3D11Device* device, ID3D11Texture2D* input, const Config& config, LogFn log);
    void shutdown();

    /// Encode what `input` holds now (the caller has queued its draw on the
    /// same device). The next frame is an IDR if `forceIdr` or requested.
    bool encode(bool forceIdr, std::vector<uint8_t>& outNalData, bool& outIsIdr);

    void requestIdr();

    /// Set a new target bitrate (the engine's adaptive bitrate, sleep mode,
    /// the headset's CONFIG_UPDATE). Thread-safe; applied before the next
    /// encode, without an IDR.
    void requestBitrate(uint32_t bitrateBps);

    bool isInitialized() const { return m_initialized; }

    /// Update gaze position for foveated encoding.
    /// Coordinates are normalized (0-1). Called from tracking data receiver.
    void setGaze(float gazeX, float gazeY, bool valid);

    /// Enable/disable foveated encoding. Takes effect at the next init():
    /// NVENC reads the QP delta map only when the session was created with
    /// qpMapMode = NV_ENC_QP_MAP_DELTA.
    void setFoveatedEnabled(bool enabled) { m_foveatedEnabled = enabled; }

    /// Get the current foveated encoding mode description.
    /// NVENC ROI is out-of-scope for v3.0 (see TODOS.md); the only active
    /// path is the per-CTU QP delta map.
    const char* foveatedModeStr() const {
        return m_foveatedEnabled ? "qp_delta_map" : "disabled";
    }

private:
    // NVENC session
    void* m_encoder = nullptr;

    // D3D11 resources
    ComPtr<ID3D11Device> m_device;
    ComPtr<ID3D11Texture2D> m_inputTexture; // Registered with NVENC
    LogFn m_log = nullptr;

    // NVENC resources
    NV_ENCODE_API_FUNCTION_LIST m_nvencFns{};
    void* m_nvencLib = nullptr;
    NV_ENC_REGISTERED_PTR m_registeredResource = nullptr;
    NV_ENC_OUTPUT_PTR m_bitstreamBuffer = nullptr;

    // The session's parameters, kept for nvEncReconfigureEncoder.
    NV_ENC_INITIALIZE_PARAMS m_initParams{};
    NV_ENC_CONFIG m_encodeConfig{};
    fvp_nvenc::StreamSettings m_settings;
    std::atomic<uint32_t> m_pendingBitrate{0};  // 0 = no change requested
    uint32_t m_reconfigureFailures = 0;

    // Encoder state
    Config m_config;
    bool m_initialized = false;
    uint32_t m_frameCount = 0;
    uint32_t m_idrInterval = 180;

    // Foveated encoding state
    bool m_foveatedEnabled = false;
    bool m_qpMapActive = false; // session created with NV_ENC_QP_MAP_DELTA
    std::atomic<float> m_gazeX{0.5f};
    std::atomic<float> m_gazeY{0.5f};
    std::atomic<bool> m_gazeValid{false};
    std::vector<int8_t> m_qpDeltaMap; // Per-CTB QP delta map for foveated encoding
    uint32_t m_ctuCols = 0;
    uint32_t m_ctuRows = 0;

    void computeQpDeltaMap(float gazeX, float gazeY);

    bool loadNvencApi();
    bool createEncoderSession();
    bool createResources();
    void applyPendingBitrate();
    void log(const char* format, ...);
};
