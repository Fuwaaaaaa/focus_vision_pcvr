#include "nvenc_encoder.h"
#include <cstdio>
#include <cstring>
#include <atomic>

// NVENC API is loaded at runtime from nvEncodeAPI64.dll.
// These are the minimal type definitions needed to call the API
// without requiring the full SDK headers as a build dependency.
//
// Reference: NVIDIA Video Codec SDK - nvEncodeAPI.h
// https://developer.nvidia.com/video-codec-sdk

// NVENC GUIDs
static const GUID NV_ENC_CODEC_H264_GUID =
    { 0x6BC82762, 0x4E63, 0x4CA4, { 0xAA, 0x85, 0x1E, 0xA8, 0x9E, 0x37, 0x8C, 0x9B } };
static const GUID NV_ENC_CODEC_HEVC_GUID =
    { 0x790CDC88, 0x4522, 0x4D7B, { 0x94, 0x25, 0xBD, 0xA9, 0x97, 0x5F, 0x76, 0x03 } };
static const GUID NV_ENC_PRESET_P4_GUID =
    { 0xFC0A8D3E, 0x4545, 0x4B14, { 0xA1, 0x41, 0xB4, 0xB6, 0x72, 0xFD, 0x42, 0x27 } };

// NVENC API version
#define NVENCAPI_MAJOR_VERSION 12
#define NVENCAPI_MINOR_VERSION 2
#define NVENCAPI_VERSION (NVENCAPI_MAJOR_VERSION | (NVENCAPI_MINOR_VERSION << 24))

// Minimal NVENC structures
// Full definitions in the NVIDIA Video Codec SDK nvEncodeAPI.h
// We define only what we use to keep the build self-contained.

typedef enum {
    NV_ENC_SUCCESS = 0,
    NV_ENC_ERR_GENERIC = 1,
} NVENCSTATUS;

typedef enum {
    NV_ENC_DEVICE_TYPE_DIRECTX = 0,
} NV_ENC_DEVICE_TYPE;

typedef enum {
    NV_ENC_BUFFER_FORMAT_NV12 = 0x00000001,
} NV_ENC_BUFFER_FORMAT;

typedef enum {
    NV_ENC_PIC_TYPE_IDR = 4,
} NV_ENC_PIC_TYPE;

typedef enum {
    NV_ENC_TUNING_INFO_LOW_LATENCY = 2,
} NV_ENC_TUNING_INFO;

typedef enum {
    NV_ENC_PARAMS_RC_CBR = 2,
} NV_ENC_PARAMS_RC_MODE;

// ---- Placeholder NVENC implementation ----
// The actual NVENC integration requires the NVIDIA Video Codec SDK.
// This file provides the structure and API surface. The real encode()
// path will be activated once the SDK is available.
//
// For now, encode() generates a synthetic NAL unit from the frame data
// to validate the full pipeline (C++ -> C ABI -> Rust RTP -> UDP).

static std::atomic<bool> s_idrRequested{false};

NvencEncoder::~NvencEncoder() {
    shutdown();
}

bool NvencEncoder::init(ID3D11Device* device, const Config& config) {
    if (m_initialized) return true;

    m_device = device;
    device->GetImmediateContext(&m_context);
    m_config = config;

    // Try to load NVENC DLL
    if (!loadNvencApi()) {
        // NVENC not available — fall back to test pattern mode.
        // This allows the pipeline to work end-to-end without a GPU encoder.
        char buf[256];
        snprintf(buf, sizeof(buf),
            "NvencEncoder: nvEncodeAPI64.dll not found. "
            "Running in test-pattern mode (no real encoding).\n");
        OutputDebugStringA(buf);
    }

    if (!createNv12Texture()) {
        return false;
    }

    m_initialized = true;
    m_frameCount = 0;
    return true;
}

void NvencEncoder::shutdown() {
    if (!m_initialized) return;

    // Destroy NVENC session
    if (m_encoder && m_nvenc) {
        // nvenc->nvEncDestroyEncoder(m_encoder);
        m_encoder = nullptr;
    }

    // Free NVENC library
    if (m_nvencLib) {
        FreeLibrary(static_cast<HMODULE>(m_nvencLib));
        m_nvencLib = nullptr;
    }

    delete m_nvenc;
    m_nvenc = nullptr;

    m_nv12Texture.Reset();
    m_stagingTexture.Reset();
    m_context.Reset();
    m_device.Reset();
    m_initialized = false;
}

bool NvencEncoder::encode(ID3D11Texture2D* srcTexture,
                          bool forceIdr,
                          std::vector<uint8_t>& outNalData,
                          bool& outIsIdr) {
    if (!m_initialized) return false;

    // Determine if this frame should be IDR
    bool isIdr = forceIdr || s_idrRequested.exchange(false) ||
                 (m_frameCount % m_idrInterval == 0);
    outIsIdr = isIdr;

    m_frameCount++;

    // --- Real NVENC path (when SDK is available) ---
    // 1. Convert BGRA texture to NV12 via compute shader or GPU copy
    // 2. Register NV12 texture as NVENC input resource
    // 3. Call nvEncEncodePicture() with IDR flag if needed
    // 4. Lock output bitstream, copy NAL data
    // 5. Unlock and return
    //
    // --- Test pattern path (current) ---
    // Generate a synthetic H.265 NAL unit for pipeline validation.
    // This allows testing the full path: C++ -> fvp_submit_encoded_nal() ->
    // Rust RTP -> FEC -> UDP without requiring real GPU encoding.

    if (m_encoder) {
        // TODO: Real NVENC encode path
        // This branch activates when the NVIDIA Video Codec SDK is integrated.
        return false;
    }

    // Test pattern: generate a fake NAL unit
    // H.265 NAL header: forbidden_zero_bit(1) + nal_unit_type(6) + nuh_layer_id(6) + nuh_temporal_id_plus1(3)
    // IDR: nal_unit_type = 19 (IDR_W_RADL), Non-IDR: nal_unit_type = 1 (TRAIL_R)
    outNalData.clear();

    // Start code prefix (Annex B format)
    outNalData.push_back(0x00);
    outNalData.push_back(0x00);
    outNalData.push_back(0x00);
    outNalData.push_back(0x01);

    if (isIdr) {
        // IDR_W_RADL: nal_unit_type = 19 -> (19 << 1) = 0x26, layer_id=0, tid=1 -> 0x01
        outNalData.push_back(0x26);
        outNalData.push_back(0x01);
    } else {
        // TRAIL_R: nal_unit_type = 1 -> (1 << 1) = 0x02, layer_id=0, tid=1 -> 0x01
        outNalData.push_back(0x02);
        outNalData.push_back(0x01);
    }

    // Fake payload — in production this is the actual encoded slice data.
    // Use frame dimensions to generate a payload that exercises the RTP
    // packetization (needs to be larger than one MTU for multi-packet frames).
    size_t payloadSize = static_cast<size_t>(m_config.width) * m_config.height / 100;
    if (payloadSize < 256) payloadSize = 256;
    outNalData.resize(outNalData.size() + payloadSize, 0xAB);

    return true;
}

void NvencEncoder::requestIdr() {
    s_idrRequested.store(true);
}

bool NvencEncoder::loadNvencApi() {
    HMODULE lib = LoadLibraryA("nvEncodeAPI64.dll");
    if (!lib) {
        return false;
    }
    m_nvencLib = lib;

    // In full implementation:
    // 1. GetProcAddress for NvEncodeAPICreateInstance
    // 2. Fill NV_ENCODE_API_FUNCTION_LIST
    // 3. Call nvEncOpenEncodeSessionEx with D3D11 device
    // 4. Configure encoder (HEVC/H264, CBR, low-latency tuning)
    // 5. Allocate input/output buffers

    return true;
}

bool NvencEncoder::createNv12Texture() {
    if (!m_device) return false;

    // NV12 texture for BGRA->NV12 conversion output
    D3D11_TEXTURE2D_DESC nv12Desc = {};
    nv12Desc.Width = m_config.width;
    nv12Desc.Height = m_config.height;
    nv12Desc.MipLevels = 1;
    nv12Desc.ArraySize = 1;
    nv12Desc.Format = DXGI_FORMAT_NV12;
    nv12Desc.SampleDesc.Count = 1;
    nv12Desc.Usage = D3D11_USAGE_DEFAULT;
    nv12Desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;

    HRESULT hr = m_device->CreateTexture2D(&nv12Desc, nullptr, &m_nv12Texture);
    if (FAILED(hr)) {
        // NV12 may not be supported on all GPUs — this is acceptable
        // since we fall back to test pattern mode.
        return true;
    }

    // Staging texture for CPU readback (fallback path)
    D3D11_TEXTURE2D_DESC stagingDesc = nv12Desc;
    stagingDesc.Usage = D3D11_USAGE_STAGING;
    stagingDesc.BindFlags = 0;
    stagingDesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

    m_device->CreateTexture2D(&stagingDesc, nullptr, &m_stagingTexture);
    return true;
}

bool NvencEncoder::convertBgraToNv12(ID3D11Texture2D* srcBgra) {
    // In full implementation: dispatch a compute shader that converts
    // BGRA (DXGI_FORMAT_B8G8R8A8_UNORM) to NV12 on the GPU.
    //
    // Color space: BT.709
    //   Y  =  0.2126*R + 0.7152*G + 0.0722*B
    //   Cb = -0.1146*R - 0.3854*G + 0.5000*B + 128
    //   Cr =  0.5000*R - 0.4542*G - 0.0458*B + 128
    //
    // The NV12 texture is then registered as NVENC input.
    return true;
}
