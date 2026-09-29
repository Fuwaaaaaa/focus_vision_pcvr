#include "nvenc_encoder.h"
#include "qp_map.h"
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <cmath>
#include <atomic>

static std::atomic<bool> s_idrRequested{false};

NvencEncoder::~NvencEncoder() {
    shutdown();
}

void NvencEncoder::log(const char* format, ...) {
    char buf[320];
    va_list args;
    va_start(args, format);
    vsnprintf(buf, sizeof(buf), format, args);
    va_end(args);
    if (m_log) {
        m_log(buf);
    } else {
        OutputDebugStringA(buf);
        OutputDebugStringA("\n");
    }
}

bool NvencEncoder::init(ID3D11Device* device, ID3D11Texture2D* input, const Config& config, LogFn log) {
    // Idempotent reinit: if a prior session exists, tear it down fully first.
    // This covers reconfigure-on-the-fly scenarios and prevents NVENC session
    // leaks (GeForce caps concurrent sessions at 2 — a leaked session across
    // pair/unpair cycles would eventually hand the next user
    // NV_ENC_ERR_OUT_OF_MEMORY until the process exits).
    if (m_initialized) {
        shutdown();
    }

    m_log = log;
    m_device = device;
    m_inputTexture = input;
    m_config = config;

    if (!input || !loadNvencApi() || !createEncoderSession() || !createResources()) {
        // No stream rather than a fake one: the headset can't decode made-up
        // NAL units, and the old silent test-pattern fallback hid the
        // failure. The reason was logged where it happened. shutdown() frees
        // whatever was created (bitstream buffer, registered resource,
        // session, nvEncodeAPI.dll).
        shutdown();
        return false;
    }

    this->log("NVENC ready: %s %ux%u, %u Mbps", config.use_hevc ? "HEVC" : "H.264",
              config.width, config.height, config.bitrate_bps / 1'000'000);
    m_initialized = true;
    m_frameCount = 0;
    return true;
}

void NvencEncoder::shutdown() {
    // Idempotent — safe to call from the destructor, from a partial-init
    // failure path, and on an already-shutdown instance. Every field is
    // null-checked before use, so partial-init cleanup can free any
    // resources (nvEncodeAPI.dll, bitstream buffer, registered resource,
    // encoder session) that were allocated before the init failure.
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
    m_qpMapActive = false;

    if (m_nvencLib) {
        FreeLibrary(static_cast<HMODULE>(m_nvencLib));
        m_nvencLib = nullptr;
    }

    m_inputTexture.Reset();
    m_device.Reset();
    m_initialized = false;
    memset(&m_nvencFns, 0, sizeof(m_nvencFns));
}

bool NvencEncoder::encode(bool forceIdr, std::vector<uint8_t>& outNalData, bool& outIsIdr) {
    if (!m_initialized || !m_encoder) return false;

    applyPendingBitrate();

    bool isIdr = forceIdr || s_idrRequested.exchange(false) ||
                 (m_frameCount % m_idrInterval == 0);
    outIsIdr = isIdr;
    m_frameCount++;

    // Read gaze data for foveated encoding via per-CTU QP delta map.
    // NVENC ROI capability is intentionally not wired in v3.0 — see TODOS.md
    // for the rationale (cannot validate without specific hardware) and the
    // re-open condition. The QP delta path achieves ~30% bandwidth reduction.
    if (m_foveatedEnabled && m_gazeValid.load()) {
        float gx = m_gazeX.load();
        float gy = m_gazeY.load();
        computeQpDeltaMap(gx, gy);
    }

    // Map the registered input. NVENC orders this after the draw the caller
    // queued on the same device.
    NV_ENC_MAP_INPUT_RESOURCE mapInput = {};
    mapInput.version = NV_ENC_MAP_INPUT_RESOURCE_VER;
    mapInput.registeredResource = m_registeredResource;
    NVENCSTATUS st = m_nvencFns.nvEncMapInputResource(m_encoder, &mapInput);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncMapInputResource failed: %d", st);
        return false;
    }

    // Encode. enablePTD is on, so NVENC picks the picture type; an IDR
    // is requested through encodePicFlags.
    NV_ENC_PIC_PARAMS picParams = {};
    picParams.version = NV_ENC_PIC_PARAMS_VER;
    picParams.inputWidth = m_config.width;
    picParams.inputHeight = m_config.height;
    picParams.inputPitch = m_config.width;
    picParams.inputBuffer = mapInput.mappedResource;
    picParams.outputBitstream = m_bitstreamBuffer;
    picParams.bufferFmt = mapInput.mappedBufferFmt;
    picParams.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    picParams.frameIdx = m_frameCount - 1;
    picParams.inputTimeStamp = m_frameCount - 1;
    picParams.encodePicFlags = fvp_nvenc::picFlagsFor(isIdr);

    // Apply the foveated QP delta map. NVENC expects it only when the
    // session was created with qpMapMode = NV_ENC_QP_MAP_DELTA.
    if (m_qpMapActive && m_foveatedEnabled && !m_qpDeltaMap.empty()) {
        picParams.qpDeltaMap = m_qpDeltaMap.data();
        picParams.qpDeltaMapSize = static_cast<uint32_t>(m_qpDeltaMap.size());
    }

    st = m_nvencFns.nvEncEncodePicture(m_encoder, &picParams);
    m_nvencFns.nvEncUnmapInputResource(m_encoder, mapInput.mappedResource);
    if (st == NV_ENC_ERR_NEED_MORE_INPUT) {
        // B-frame delay — no output yet. We don't use B-frames for low latency,
        // but handle gracefully.
        outNalData.clear();
        return true;
    }
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncEncodePicture failed: %d", st);
        return false;
    }

    // Lock bitstream and copy NAL data
    NV_ENC_LOCK_BITSTREAM lockBitstream = {};
    lockBitstream.version = NV_ENC_LOCK_BITSTREAM_VER;
    lockBitstream.outputBitstream = m_bitstreamBuffer;
    st = m_nvencFns.nvEncLockBitstream(m_encoder, &lockBitstream);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncLockBitstream failed: %d", st);
        return false;
    }

    outNalData.resize(lockBitstream.bitstreamSizeInBytes);
    memcpy(outNalData.data(), lockBitstream.bitstreamBufferPtr,
           lockBitstream.bitstreamSizeInBytes);

    outIsIdr = (lockBitstream.pictureType == NV_ENC_PIC_TYPE_IDR);

    m_nvencFns.nvEncUnlockBitstream(m_encoder, m_bitstreamBuffer);
    return true;
}

void NvencEncoder::requestIdr() {
    s_idrRequested.store(true);
}

void NvencEncoder::requestBitrate(uint32_t bitrateBps) {
    if (bitrateBps != 0) m_pendingBitrate.store(bitrateBps);
}

void NvencEncoder::applyPendingBitrate() {
    const uint32_t bps = m_pendingBitrate.exchange(0);
    if (bps == 0 || bps == m_settings.bitrate_bps) return;

    fvp_nvenc::StreamSettings next = m_settings;
    next.bitrate_bps = bps;
    NV_ENC_CONFIG config = m_encodeConfig;
    NV_ENC_RECONFIGURE_PARAMS params = fvp_nvenc::reconfigureParams(m_initParams, config, next);
    const NVENCSTATUS st = m_nvencFns.nvEncReconfigureEncoder(m_encoder, &params);
    if (st != NV_ENC_SUCCESS) {
        // Keep encoding at the old bitrate. Adaptive bitrate asks about once
        // a second, so log the 1st, 2nd, 4th, 8th... failure only.
        m_reconfigureFailures++;
        if ((m_reconfigureFailures & (m_reconfigureFailures - 1)) == 0) {
            log("NVENC: bitrate change to %u Mbps failed: %d (%u so far)", bps / 1'000'000, st,
                m_reconfigureFailures);
        }
        return;
    }
    m_encodeConfig = config;
    m_initParams.encodeConfig = &m_encodeConfig;
    m_settings = next;
    m_config.bitrate_bps = bps;
}

void NvencEncoder::setGaze(float gazeX, float gazeY, bool valid) {
    m_gazeX.store(gazeX);
    m_gazeY.store(gazeY);
    m_gazeValid.store(valid);
}

void NvencEncoder::computeQpDeltaMap(float gazeX, float gazeY) {
    const uint32_t ctuSize = fvp_nvenc::qpMapBlockSize(m_config.use_hevc);

    // Use preset offsets from config (default: balanced = +5/+15). The frame
    // holds both eyes, so each gets its own fovea around the gaze point.
    ::computeSideBySideQpDeltaMap(
        gazeX, gazeY, m_config.eye_width, m_config.height, ctuSize,
        m_config.fovea_radius, m_config.mid_radius,
        static_cast<int8_t>(m_config.mid_qp_offset), static_cast<int8_t>(m_config.peripheral_qp_offset),
        m_qpDeltaMap, m_ctuCols, m_ctuRows);
}

bool NvencEncoder::loadNvencApi() {
    HMODULE lib = LoadLibraryA("nvEncodeAPI64.dll");
    if (!lib) {
        log("NVENC unavailable: nvEncodeAPI64.dll not found (needs an NVIDIA GPU and driver)");
        return false;
    }
    m_nvencLib = lib;

    // The driver must support the API version the header was written for,
    // or every call fails with NV_ENC_ERR_INVALID_VERSION.
    using GetMaxSupportedVersionFn = NVENCSTATUS (NVENCAPI*)(uint32_t*);
    auto getMaxVersion = reinterpret_cast<GetMaxSupportedVersionFn>(
        GetProcAddress(lib, "NvEncodeAPIGetMaxSupportedVersion"));
    uint32_t maxVersion = 0;
    if (!getMaxVersion || getMaxVersion(&maxVersion) != NV_ENC_SUCCESS
            || !fvp_nvenc::driverSupportsApi(maxVersion)) {
        log("NVENC unavailable: the NVIDIA driver supports NVENC API %u.%u, need %u.%u. "
            "Update the NVIDIA driver.",
            maxVersion >> 4, maxVersion & 0xF,
            NVENCAPI_MAJOR_VERSION, NVENCAPI_MINOR_VERSION);
        return false;
    }

    using CreateInstanceFn = NVENCSTATUS (NVENCAPI*)(NV_ENCODE_API_FUNCTION_LIST*);
    auto createInstance = reinterpret_cast<CreateInstanceFn>(
        GetProcAddress(lib, "NvEncodeAPICreateInstance"));
    if (!createInstance) {
        log("NVENC unavailable: NvEncodeAPICreateInstance not found");
        return false;
    }

    m_nvencFns.version = NV_ENCODE_API_FUNCTION_LIST_VER;
    NVENCSTATUS st = createInstance(&m_nvencFns);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC unavailable: NvEncodeAPICreateInstance failed: %d", st);
        return false;
    }

    return true;
}

bool NvencEncoder::createEncoderSession() {
    if (!m_nvencFns.nvEncOpenEncodeSessionEx) return false;

    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS sessionParams = {};
    sessionParams.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    sessionParams.deviceType = NV_ENC_DEVICE_TYPE_DIRECTX;
    sessionParams.device = m_device.Get();
    sessionParams.apiVersion = NVENCAPI_VERSION;

    NVENCSTATUS st = m_nvencFns.nvEncOpenEncodeSessionEx(&sessionParams, &m_encoder);
    if (st != NV_ENC_SUCCESS) {
        // NV_ENC_ERR_OUT_OF_MEMORY here usually means GeForce's limit on
        // concurrent sessions (OBS, a game recorder) was reached.
        log("NVENC unavailable: nvEncOpenEncodeSessionEx failed: %d", st);
        m_encoder = nullptr;
        return false;
    }

    // Start from NVIDIA's preset for this codec and tuning, then apply the
    // streaming settings. A zeroed NV_ENC_CONFIG is not a valid config.
    const GUID codecGuid = m_config.use_hevc ? NV_ENC_CODEC_HEVC_GUID : NV_ENC_CODEC_H264_GUID;
    const GUID presetGuid = NV_ENC_PRESET_P4_GUID;
    const NV_ENC_TUNING_INFO tuning = NV_ENC_TUNING_INFO_LOW_LATENCY;

    NV_ENC_PRESET_CONFIG presetConfig = {};
    presetConfig.version = NV_ENC_PRESET_CONFIG_VER;
    presetConfig.presetCfg.version = NV_ENC_CONFIG_VER;
    st = m_nvencFns.nvEncGetEncodePresetConfigEx(m_encoder, codecGuid, presetGuid, tuning,
                                                 &presetConfig);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncGetEncodePresetConfigEx failed: %d", st);
        return false;
    }

    m_encodeConfig = presetConfig.presetCfg;
    m_settings = {};
    m_settings.hevc = m_config.use_hevc;
    m_settings.bitrate_bps = m_config.bitrate_bps;
    m_settings.fps = m_config.fps;
    m_settings.full_range = m_config.full_range;
    m_settings.qp_delta_map = m_foveatedEnabled;
    fvp_nvenc::applyStreamSettings(m_encodeConfig, m_settings);

    m_initParams = {};
    m_initParams.version = NV_ENC_INITIALIZE_PARAMS_VER;
    m_initParams.encodeGUID = codecGuid;
    m_initParams.presetGUID = presetGuid;
    m_initParams.tuningInfo = tuning;
    m_initParams.encodeWidth = m_config.width;
    m_initParams.encodeHeight = m_config.height;
    m_initParams.darWidth = m_config.width;
    m_initParams.darHeight = m_config.height;
    m_initParams.maxEncodeWidth = m_config.width;
    m_initParams.maxEncodeHeight = m_config.height;
    m_initParams.frameRateNum = m_config.fps;
    m_initParams.frameRateDen = 1;
    m_initParams.enablePTD = 1; // Picture type decision by encoder
    m_initParams.encodeConfig = &m_encodeConfig;

    st = m_nvencFns.nvEncInitializeEncoder(m_encoder, &m_initParams);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncInitializeEncoder failed: %d (%s %ux%u)", st,
            m_config.use_hevc ? "HEVC" : "H.264", m_config.width, m_config.height);
        return false;
    }

    m_qpMapActive = m_settings.qp_delta_map;
    return true;
}

bool NvencEncoder::createResources() {
    if (!m_encoder) return false;

    // Register the input texture (EyeBlit's output). NV_ENC_BUFFER_FORMAT_ARGB
    // is word-ordered A8R8G8B8, i.e. the byte order of
    // DXGI_FORMAT_B8G8R8A8_UNORM.
    NV_ENC_REGISTER_RESOURCE regResource = {};
    regResource.version = NV_ENC_REGISTER_RESOURCE_VER;
    regResource.resourceType = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
    regResource.width = m_config.width;
    regResource.height = m_config.height;
    regResource.resourceToRegister = m_inputTexture.Get();
    regResource.bufferFormat = NV_ENC_BUFFER_FORMAT_ARGB;
    regResource.bufferUsage = NV_ENC_INPUT_IMAGE;

    NVENCSTATUS st = m_nvencFns.nvEncRegisterResource(m_encoder, &regResource);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncRegisterResource failed: %d", st);
        return false;
    }
    m_registeredResource = regResource.registeredResource;

    // Create output bitstream buffer
    NV_ENC_CREATE_BITSTREAM_BUFFER createBitstream = {};
    createBitstream.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

    st = m_nvencFns.nvEncCreateBitstreamBuffer(m_encoder, &createBitstream);
    if (st != NV_ENC_SUCCESS) {
        log("NVENC: nvEncCreateBitstreamBuffer failed: %d", st);
        return false;
    }
    m_bitstreamBuffer = createBitstream.bitstreamBuffer;

    return true;
}
