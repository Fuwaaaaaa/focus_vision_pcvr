#pragma once

#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>
#include <vector>
#include <string>

using Microsoft::WRL::ComPtr;

// Forward declaration — full NVENC SDK headers loaded at runtime via DLL.
// This avoids requiring the SDK as a build dependency.
struct NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS;
struct NV_ENCODE_API_FUNCTION_LIST;

/**
 * NVENC hardware video encoder.
 *
 * Handles D3D11 texture -> NV12 conversion (compute shader) and
 * H.265/H.264 encoding via NVIDIA's Video Codec SDK, loaded at runtime.
 *
 * Architecture decision (eng review #1): encoding runs entirely in C++.
 * Only the resulting NAL byte array crosses the C ABI into Rust for
 * RTP packetization.
 */
class NvencEncoder {
public:
    struct Config {
        uint32_t width = 1832;
        uint32_t height = 1920;
        uint32_t fps = 90;
        uint32_t bitrate_bps = 80'000'000;
        bool use_hevc = true; // H.265 by default, H.264 as fallback
    };

    NvencEncoder() = default;
    ~NvencEncoder();

    // Non-copyable
    NvencEncoder(const NvencEncoder&) = delete;
    NvencEncoder& operator=(const NvencEncoder&) = delete;

    /// Initialize NVENC. Loads nvEncodeAPI64.dll at runtime.
    /// Returns false if NVIDIA GPU or NVENC is unavailable.
    bool init(ID3D11Device* device, const Config& config);

    /// Shut down encoder and release resources.
    void shutdown();

    /// Encode a D3D11 texture (BGRA/RGBA format) into H.265 NAL units.
    /// The source texture is copied internally — caller can reuse it immediately.
    ///
    /// `forceIdr`: if true, this frame will be encoded as an IDR keyframe.
    ///
    /// On success, returns true and fills `outNalData` with encoded bytes.
    /// `outIsIdr` is set to true if the frame was encoded as IDR (forced or periodic).
    bool encode(ID3D11Texture2D* srcTexture,
                bool forceIdr,
                std::vector<uint8_t>& outNalData,
                bool& outIsIdr);

    /// Request that the next encode produces an IDR keyframe.
    /// Thread-safe — can be called from the TCP control thread.
    void requestIdr();

    bool isInitialized() const { return m_initialized; }

private:
    // NVENC session — opaque, managed via NVENC API function pointers
    void* m_encoder = nullptr;

    // D3D11 resources
    ComPtr<ID3D11Device> m_device;
    ComPtr<ID3D11DeviceContext> m_context;
    ComPtr<ID3D11Texture2D> m_nv12Texture;     // NV12 conversion target
    ComPtr<ID3D11Texture2D> m_stagingTexture;   // CPU-readable copy for fallback

    // NVENC API function table (loaded from DLL)
    NV_ENCODE_API_FUNCTION_LIST* m_nvenc = nullptr;
    void* m_nvencLib = nullptr; // HMODULE

    // Encoder state
    Config m_config;
    bool m_initialized = false;
    uint32_t m_frameCount = 0;
    uint32_t m_idrInterval = 180; // IDR every 2 seconds at 90fps

    // Internal helpers
    bool loadNvencApi();
    bool createNv12Texture();
    bool convertBgraToNv12(ID3D11Texture2D* srcBgra);
};
