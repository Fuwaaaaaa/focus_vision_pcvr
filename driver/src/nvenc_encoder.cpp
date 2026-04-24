#include "nvenc_encoder.h"
#include "qp_map.h"
#include <cstdio>
#include <cstring>
#include <cmath>
#include <atomic>

static std::atomic<bool> s_idrRequested{false};

NvencEncoder::~NvencEncoder() {
    shutdown();
}

bool NvencEncoder::init(ID3D11Device* device, const Config& config) {
    // Idempotent reinit: if a prior session exists, tear it down fully first.
    // This covers reconfigure-on-the-fly scenarios and prevents NVENC session
    // leaks (GeForce caps concurrent sessions at 2 — a leaked session across
    // pair/unpair cycles would eventually hand the next user
    // NV_ENC_ERR_OUT_OF_MEMORY until the process exits).
    if (m_initialized) {
        shutdown();
    }

    m_device = device;
    device->GetImmediateContext(&m_context);
    m_config = config;

    if (loadNvencApi() && createEncoderSession() && createResources()) {
        char buf[256];
        snprintf(buf, sizeof(buf),
            "NvencEncoder: Real NVENC initialized (%s, %ux%u, %u Mbps)\n",
            config.use_hevc ? "HEVC" : "H264",
            config.width, config.height, config.bitrate_bps / 1'000'000);
        OutputDebugStringA(buf);
    } else {
        // NVENC not available — test pattern mode.
        OutputDebugStringA(
            "NvencEncoder: NVENC unavailable. Running in test-pattern mode.\n");
        // Tear down any partially-created NVENC state (bitstream buffer,
        // registered resource, encoder session, loaded nvEncodeAPI.dll).
        // Funnelling through shutdown() keeps the cleanup list in one place
        // and avoids drift between partial-init and normal-teardown paths.
        // shutdown() intentionally resets m_device/m_context/m_inputTexture
        // too, so we re-capture them after the call before returning.
        shutdown();
        m_device = device;
        device->GetImmediateContext(&m_context);
        m_config = config;
    }

    m_initialized = true;
    m_frameCount = 0;
    return true;
}

void NvencEncoder::shutdown() {
    // Idempotent — safe to call from the destructor, from a partial-init
    // failure path, and on an already-shutdown instance. Every field is
    // null-checked before use, so the absence of the old
    // `if (!m_initialized) return;` early exit is intentional: partial-init
    // cleanup must be able to free any resources (nvEncodeAPI.dll,
    // bitstream buffer, registered resource, encoder session) that were
    // allocated before the init failure.
    if (m_encoder) {
        if (m_bitstreamBuffer && m_nvencFns.nvEncDestroyBitstreamBuffer)
            m_nvencFns.nvEncDestroyBitstreamBuffer(m_encoder, m_bitstreamBuffer);
        if (m_registeredResource && m_nvencFns.nvEncUnregisterResource)
            m_nvencFns.nvEncUnregisterResource(m_encoder, m_registeredResource);
        if (m_nvencFns.nvEncDestroyEncoder)
            m_nvencFns.nvEncDestroyEncoder(m_encoder);
        m_encoder = nullptr;
    }

    m_bitstreamBuffer = nullptr;
    m_registeredResource = nullptr;

    if (m_nvencLib) {
        FreeLibrary(static_cast<HMODULE>(m_nvencLib));
        m_nvencLib = nullptr;
    }

    m_inputTexture.Reset();
    m_context.Reset();
    m_device.Reset();
    m_initialized = false;
    memset(&m_nvencFns, 0, sizeof(m_nvencFns));
}

bool NvencEncoder::encode(ID3D11Texture2D* srcTexture,
                          bool forceIdr,
                          std::vector<uint8_t>& outNalData,
                          bool& outIsIdr) {
    if (!m_initialized) return false;

    bool isIdr = forceIdr || s_idrRequested.exchange(false) ||
                 (m_frameCount % m_idrInterval == 0);
    outIsIdr = isIdr;
    m_frameCount++;

    // Read gaze data for foveated encoding.
    // Uses ROI if supported, falls back to QP delta map otherwise.
    if (m_foveatedEnabled && m_gazeValid.load()) {
        float gx = m_gazeX.load();
        float gy = m_gazeY.load();
        if (m_roiSupported) {
            // TODO: NVENC ROI path — set per-region quality via ROI API
            // For now, fall through to QP delta map
        }
        // Fallback: QP delta map (always available)
        computeQpDeltaMap(gx, gy);
    }

    // --- Real NVENC path ---
    if (m_encoder) {
        // Copy source texture to our registered input texture
        if (srcTexture) {
            m_context->CopyResource(m_inputTexture.Get(), srcTexture);
            m_context->Flush();
        }

        // Map the registered resource
        NV_ENC_MAP_INPUT_RESOURCE mapInput = {};
        mapInput.version = NVENCAPI_STRUCT_VERSION(NV_ENC_MAP_INPUT_RESOURCE, 4);
        mapInput.registeredResource = m_registeredResource;
        NVENCSTATUS st = m_nvencFns.nvEncMapInputResource(m_encoder, &mapInput);
        if (st != NV_ENC_SUCCESS) {
            OutputDebugStringA("NvencEncoder: nvEncMapInputResource failed\n");
            return false;
        }

        // Encode
        NV_ENC_PIC_PARAMS picParams = {};
        picParams.version = NVENCAPI_STRUCT_VERSION(NV_ENC_PIC_PARAMS, 6);
        picParams.inputWidth = m_config.width;
        picParams.inputHeight = m_config.height;
        picParams.inputPitch = m_config.width;
        picParams.inputBuffer = mapInput.mappedResource;
        picParams.outputBitstream = m_bitstreamBuffer;
        picParams.bufferFmt = NV_ENC_BUFFER_FORMAT_ARGB;
        picParams.pictureStruct = 1; // Frame
        picParams.frameIdx = m_frameCount - 1;
        picParams.inputTimeStamp = m_frameCount - 1;

        if (isIdr) {
            picParams.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR;
            picParams.pictureType = NV_ENC_PIC_TYPE_IDR;
        }

        // Apply foveated QP delta map if computed
        if (m_foveatedEnabled && !m_qpDeltaMap.empty()) {
            picParams.qpDeltaMap = m_qpDeltaMap.data();
            picParams.qpDeltaMapSize = static_cast<uint32_t>(m_qpDeltaMap.size());
        }

        st = m_nvencFns.nvEncEncodePicture(m_encoder, &picParams);
        if (st != NV_ENC_SUCCESS && st != NV_ENC_ERR_NEED_MORE_INPUT) {
            m_nvencFns.nvEncUnmapInputResource(m_encoder, mapInput.mappedResource);
            char buf[128];
            snprintf(buf, sizeof(buf), "NvencEncoder: nvEncEncodePicture failed: %d\n", st);
            OutputDebugStringA(buf);
            return false;
        }

        // Unmap input
        m_nvencFns.nvEncUnmapInputResource(m_encoder, mapInput.mappedResource);

        if (st == NV_ENC_ERR_NEED_MORE_INPUT) {
            // B-frame delay — no output yet. We don't use B-frames for low latency,
            // but handle gracefully.
            outNalData.clear();
            return true;
        }

        // Lock bitstream and copy NAL data
        NV_ENC_LOCK_BITSTREAM lockBitstream = {};
        lockBitstream.version = NVENCAPI_STRUCT_VERSION(NV_ENC_LOCK_BITSTREAM, 2);
        lockBitstream.outputBitstream = m_bitstreamBuffer;
        st = m_nvencFns.nvEncLockBitstream(m_encoder, &lockBitstream);
        if (st != NV_ENC_SUCCESS) {
            OutputDebugStringA("NvencEncoder: nvEncLockBitstream failed\n");
            return false;
        }

        outNalData.resize(lockBitstream.bitstreamSizeInBytes);
        memcpy(outNalData.data(), lockBitstream.bitstreamBufferPtr,
               lockBitstream.bitstreamSizeInBytes);

        outIsIdr = (lockBitstream.pictureType == NV_ENC_PIC_TYPE_IDR);

        m_nvencFns.nvEncUnlockBitstream(m_encoder, m_bitstreamBuffer);
        return true;
    }

    // --- Test pattern fallback ---
    generateTestPattern(isIdr, outNalData);
    return true;
}

void NvencEncoder::requestIdr() {
    s_idrRequested.store(true);
}

void NvencEncoder::setGaze(float gazeX, float gazeY, bool valid) {
    m_gazeX.store(gazeX);
    m_gazeY.store(gazeY);
    m_gazeValid.store(valid);
}

bool NvencEncoder::queryRoiCapability() {
    // TODO: Query NVENC for ROI support via nvEncGetEncodeCaps()
    // NV_ENC_CAPS_SUPPORT_EMPHASIS_LEVEL_MAP or similar.
    // For now, always return false (ROI not supported).
    // When NVENC SDK 12.x ROI is available, query here and return true.
    //
    // Fallback behavior: use QP delta map (already implemented and tested).
    m_roiSupported = false;
    return m_roiSupported;
}

void NvencEncoder::computeQpDeltaMap(float gazeX, float gazeY) {
    const uint32_t ctuSize = m_config.use_hevc ? 64 : 16;
    computeCtuGrid(m_config.width, m_config.height, ctuSize, m_ctuCols, m_ctuRows);

    // Use preset offsets from config (default: balanced = +5/+15)
    ::computeQpDeltaMap(
        gazeX, gazeY, m_ctuCols, m_ctuRows,
        m_config.fovea_radius, m_config.mid_radius,
        m_config.mid_qp_offset, m_config.peripheral_qp_offset,
        m_qpDeltaMap);
}

bool NvencEncoder::loadNvencApi() {
    HMODULE lib = LoadLibraryA("nvEncodeAPI64.dll");
    if (!lib) return false;
    m_nvencLib = lib;

    auto createInstance = (PFN_NvEncodeAPICreateInstance)
        GetProcAddress(lib, "NvEncodeAPICreateInstance");
    if (!createInstance) {
        OutputDebugStringA("NvencEncoder: NvEncodeAPICreateInstance not found\n");
        FreeLibrary(lib);
        m_nvencLib = nullptr;
        return false;
    }

    m_nvencFns.version = NVENCAPI_STRUCT_VERSION(NV_ENCODE_API_FUNCTION_LIST, 2);
    NVENCSTATUS st = createInstance(&m_nvencFns);
    if (st != NV_ENC_SUCCESS) {
        char buf[128];
        snprintf(buf, sizeof(buf),
            "NvencEncoder: NvEncodeAPICreateInstance failed: %d\n", st);
        OutputDebugStringA(buf);
        return false;
    }

    return true;
}

bool NvencEncoder::createEncoderSession() {
    if (!m_nvencFns.nvEncOpenEncodeSessionEx) return false;

    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS sessionParams = {};
    sessionParams.version = NVENCAPI_STRUCT_VERSION(NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS, 1);
    sessionParams.deviceType = NV_ENC_DEVICE_TYPE_DIRECTX;
    sessionParams.device = m_device.Get();
    sessionParams.apiVersion = NVENCAPI_VERSION;

    NVENCSTATUS st = m_nvencFns.nvEncOpenEncodeSessionEx(&sessionParams, &m_encoder);
    if (st != NV_ENC_SUCCESS) {
        char buf[128];
        snprintf(buf, sizeof(buf),
            "NvencEncoder: nvEncOpenEncodeSessionEx failed: %d\n", st);
        OutputDebugStringA(buf);
        m_encoder = nullptr;
        return false;
    }

    // Configure encoder
    NV_ENC_CONFIG encConfig = {};
    encConfig.version = NVENCAPI_STRUCT_VERSION(NV_ENC_CONFIG, 8);
    encConfig.gopLength = m_idrInterval;
    encConfig.frameIntervalP = 1; // No B-frames (low latency)
    encConfig.rcParams.rateControlMode = NV_ENC_PARAMS_RC_CBR_LOWDELAY_HQ;
    encConfig.rcParams.averageBitRate = m_config.bitrate_bps;
    encConfig.rcParams.maxBitRate = m_config.bitrate_bps;
    if (m_foveatedEnabled) {
        encConfig.rcParams.qpMapMode = NV_ENC_QP_MAP_DELTA;
    }

    // Set VUI (Video Usability Information) parameters for color space signaling.
    // This tells the decoder whether the stream is full range (0-255) or limited (16-235).
    if (m_config.use_hevc) {
        auto& vui = encConfig.encodeCodecConfig.hevcConfig.hevcVUIParameters;
        vui.videoSignalTypePresentFlag = 1;
        vui.videoFormat = 5; // Unspecified
        vui.videoFullRangeFlag = m_config.full_range ? 1 : 0;
        vui.colourDescriptionPresentFlag = 1;
        vui.colourPrimaries = 1;            // BT.709
        vui.transferCharacteristics = 1;    // BT.709
        vui.matrixCoeffs = 1;               // BT.709
    } else {
        auto& vui = encConfig.encodeCodecConfig.h264Config.h264VUIParameters;
        vui.videoSignalTypePresentFlag = 1;
        vui.videoFormat = 5;
        vui.videoFullRangeFlag = m_config.full_range ? 1 : 0;
        vui.colourDescriptionPresentFlag = 1;
        vui.colourPrimaries = 1;
        vui.transferCharacteristics = 1;
        vui.matrixCoeffs = 1;
    }

    NV_ENC_INITIALIZE_PARAMS initParams = {};
    initParams.version = NVENCAPI_STRUCT_VERSION(NV_ENC_INITIALIZE_PARAMS, 5);
    initParams.encodeGUID = m_config.use_hevc ? NV_ENC_CODEC_HEVC_GUID : NV_ENC_CODEC_H264_GUID;
    initParams.presetGUID = NV_ENC_PRESET_P4_GUID;
    initParams.encodeWidth = m_config.width;
    initParams.encodeHeight = m_config.height;
    initParams.darWidth = m_config.width;
    initParams.darHeight = m_config.height;
    initParams.frameRateNum = m_config.fps;
    initParams.frameRateDen = 1;
    initParams.enablePTD = 1; // Picture type decision by encoder
    initParams.tuningInfo = NV_ENC_TUNING_INFO_LOW_LATENCY;
    initParams.encodeConfig = &encConfig;

    st = m_nvencFns.nvEncInitializeEncoder(m_encoder, &initParams);
    if (st != NV_ENC_SUCCESS) {
        char buf[128];
        snprintf(buf, sizeof(buf),
            "NvencEncoder: nvEncInitializeEncoder failed: %d\n", st);
        OutputDebugStringA(buf);
        return false;
    }

    // Query ROI capability after encoder is initialized
    queryRoiCapability();

    return true;
}

bool NvencEncoder::createResources() {
    if (!m_encoder) return false;

    // Create input texture that NVENC will read from.
    // We use BGRA format (matching SteamVR's compositor output).
    D3D11_TEXTURE2D_DESC desc = {};
    desc.Width = m_config.width;
    desc.Height = m_config.height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET;

    HRESULT hr = m_device->CreateTexture2D(&desc, nullptr, &m_inputTexture);
    if (FAILED(hr)) {
        OutputDebugStringA("NvencEncoder: Failed to create input texture\n");
        return false;
    }

    // Register the D3D11 texture with NVENC
    NV_ENC_REGISTER_RESOURCE regResource = {};
    regResource.version = NVENCAPI_STRUCT_VERSION(NV_ENC_REGISTER_RESOURCE, 3);
    regResource.resourceType = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
    regResource.width = m_config.width;
    regResource.height = m_config.height;
    regResource.resourceToRegister = m_inputTexture.Get();
    regResource.bufferFormat = NV_ENC_BUFFER_FORMAT_ARGB;
    regResource.bufferUsage = 0; // encoder input

    NVENCSTATUS st = m_nvencFns.nvEncRegisterResource(m_encoder, &regResource);
    if (st != NV_ENC_SUCCESS) {
        char buf[128];
        snprintf(buf, sizeof(buf),
            "NvencEncoder: nvEncRegisterResource failed: %d\n", st);
        OutputDebugStringA(buf);
        return false;
    }
    m_registeredResource = regResource.registeredResource;

    // Create output bitstream buffer
    NV_ENC_CREATE_BITSTREAM_BUFFER createBitstream = {};
    createBitstream.version = NVENCAPI_STRUCT_VERSION(NV_ENC_CREATE_BITSTREAM_BUFFER, 1);

    st = m_nvencFns.nvEncCreateBitstreamBuffer(m_encoder, &createBitstream);
    if (st != NV_ENC_SUCCESS) {
        OutputDebugStringA("NvencEncoder: nvEncCreateBitstreamBuffer failed\n");
        return false;
    }
    m_bitstreamBuffer = createBitstream.bitstreamBuffer;

    return true;
}

void NvencEncoder::generateTestPattern(bool isIdr, std::vector<uint8_t>& outNalData) {
    outNalData.clear();

    // Annex B start code
    outNalData.push_back(0x00);
    outNalData.push_back(0x00);
    outNalData.push_back(0x00);
    outNalData.push_back(0x01);

    if (isIdr) {
        outNalData.push_back(0x26); // IDR_W_RADL: (19 << 1)
        outNalData.push_back(0x01); // layer_id=0, tid=1
    } else {
        outNalData.push_back(0x02); // TRAIL_R: (1 << 1)
        outNalData.push_back(0x01);
    }

    size_t payloadSize = static_cast<size_t>(m_config.width) * m_config.height / 100;
    if (payloadSize < 256) payloadSize = 256;
    outNalData.resize(outNalData.size() + payloadSize, 0xAB);
}
