#pragma once

#include <d3d11.h>
#include <wrl/client.h>
#include <cstdint>
#include <vector>
#include <atomic>

using Microsoft::WRL::ComPtr;

// ============================================================
// NVENC API inline type definitions (from nvEncodeAPI.h v12.2)
// Defined inline to avoid requiring the SDK as a build dependency.
// The DLL is loaded at runtime via LoadLibrary.
// ============================================================

typedef int32_t NVENCSTATUS;
#define NV_ENC_SUCCESS 0
#define NV_ENC_ERR_NO_ENCODE_DEVICE 2
#define NV_ENC_ERR_UNSUPPORTED_DEVICE 3
#define NV_ENC_ERR_INVALID_ENCODERDEVICE 6
#define NV_ENC_ERR_INVALID_VERSION 12
#define NV_ENC_ERR_OUT_OF_MEMORY 13
#define NV_ENC_ERR_ENCODER_NOT_INITIALIZED 14
#define NV_ENC_ERR_NEED_MORE_INPUT 26

#define NV_ENC_DEVICE_TYPE_DIRECTX 0

#define NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX 0

#define NV_ENC_BUFFER_FORMAT_NV12       0x00000001
#define NV_ENC_BUFFER_FORMAT_ARGB       0x00000020

#define NV_ENC_PIC_TYPE_P     0
#define NV_ENC_PIC_TYPE_B     1
#define NV_ENC_PIC_TYPE_I     2
#define NV_ENC_PIC_TYPE_IDR   3
#define NV_ENC_PIC_TYPE_SKIPPED 8

#define NV_ENC_PIC_FLAG_FORCEIDR 4

#define NV_ENC_PARAMS_RC_CBR         2
#define NV_ENC_PARAMS_RC_CBR_LOWDELAY_HQ 8
#define NV_ENC_TUNING_INFO_LOW_LATENCY   2

// QP map modes for foveated encoding
#define NV_ENC_QP_MAP_DISABLED  0
#define NV_ENC_QP_MAP_EMPHASIS  1
#define NV_ENC_QP_MAP_DELTA     2

#define NVENCAPI_MAJOR_VERSION 12
#define NVENCAPI_MINOR_VERSION 2
#define NVENCAPI_VERSION (NVENCAPI_MAJOR_VERSION | (NVENCAPI_MINOR_VERSION << 24))
#define NVENCAPI_STRUCT_VERSION(typeName, ver) \
    (uint32_t)(sizeof(typeName) | ((ver) << 16) | (NVENCAPI_VERSION << 24))

// GUIDs
static const GUID NV_ENC_CODEC_H264_GUID =
    { 0x6BC82762, 0x4E63, 0x4CA4, { 0xAA, 0x85, 0x1E, 0xA8, 0x9E, 0x37, 0x8C, 0x9B } };
static const GUID NV_ENC_CODEC_HEVC_GUID =
    { 0x790CDC88, 0x4522, 0x4D7B, { 0x94, 0x25, 0xBD, 0xA9, 0x97, 0x5F, 0x76, 0x03 } };
static const GUID NV_ENC_PRESET_P4_GUID =  // Balanced preset
    { 0xFC0A8D3E, 0x4545, 0x4B14, { 0xA1, 0x41, 0xB4, 0xB6, 0x72, 0xFD, 0x42, 0x27 } };

// ---- Structures ----

struct NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS {
    uint32_t version;
    uint32_t deviceType;
    void*    device;
    uint32_t reserved;
    uint32_t apiVersion;
    uint32_t reserved1[253];
    void*    reserved2[64];
};

struct NV_ENC_PRESET_CONFIG;  // Forward; not directly used

struct NV_ENC_CONFIG_HEVC {
    uint32_t level;
    uint32_t tier;
    uint32_t reserved[254];
};

struct NV_ENC_CONFIG_H264 {
    uint32_t reserved[256];
};

struct NV_ENC_RC_PARAMS {
    uint32_t version;
    uint32_t rateControlMode;
    uint32_t reserved1[2];
    uint32_t averageBitRate;
    uint32_t maxBitRate;
    uint32_t vbvBufferSize;
    uint32_t vbvInitialDelay;
    uint32_t reserved2a[2];
    uint32_t qpMapMode;      // NV_ENC_QP_MAP_DELTA for foveated encoding
    uint32_t reserved2[245];
};

struct NV_ENC_CODEC_CONFIG {
    union {
        NV_ENC_CONFIG_HEVC hevcConfig;
        NV_ENC_CONFIG_H264 h264Config;
    };
};

struct NV_ENC_CONFIG {
    uint32_t         version;
    GUID             profileGUID;
    uint32_t         gopLength;
    int32_t          frameIntervalP;
    uint32_t         monoChromeEncoding;
    uint32_t         frameFieldMode;
    uint32_t         mvPrecision;
    NV_ENC_RC_PARAMS rcParams;
    NV_ENC_CODEC_CONFIG encodeCodecConfig;
    uint32_t         reserved[278];
    void*            reserved2[64];
};

struct NV_ENC_INITIALIZE_PARAMS {
    uint32_t version;
    GUID     encodeGUID;
    GUID     presetGUID;
    uint32_t encodeWidth;
    uint32_t encodeHeight;
    uint32_t darWidth;
    uint32_t darHeight;
    uint32_t frameRateNum;
    uint32_t frameRateDen;
    uint32_t enableEncodeAsync;
    uint32_t enablePTD;
    uint32_t reportSliceOffsets;
    uint32_t enableSubFrameWrite;
    uint32_t enableExternalMEHints;
    uint32_t enableMEOnlyMode;
    uint32_t enableWeightedPrediction;
    uint32_t enableOutputInVidmem;
    uint32_t reserved1;
    uint32_t privDataSize;
    void*    privData;
    NV_ENC_CONFIG* encodeConfig;
    uint32_t maxEncodeWidth;
    uint32_t maxEncodeHeight;
    uint32_t reserved2[2];
    uint32_t tuningInfo;
    uint32_t reserved[286];
    void*    reserved3[64];
};

struct NV_ENC_INPUT_PTR_S;
typedef NV_ENC_INPUT_PTR_S* NV_ENC_INPUT_PTR;
struct NV_ENC_OUTPUT_PTR_S;
typedef NV_ENC_OUTPUT_PTR_S* NV_ENC_OUTPUT_PTR;
struct NV_ENC_REGISTERED_PTR_S;
typedef NV_ENC_REGISTERED_PTR_S* NV_ENC_REGISTERED_PTR;

struct NV_ENC_REGISTER_RESOURCE {
    uint32_t version;
    uint32_t resourceType;
    uint32_t width;
    uint32_t height;
    uint32_t pitch;
    uint32_t subResourceIndex;
    void*    resourceToRegister;
    NV_ENC_REGISTERED_PTR registeredResource;
    uint32_t bufferFormat;
    uint32_t bufferUsage;
    uint32_t reserved[247];
    void*    reserved2[62];
};

struct NV_ENC_MAP_INPUT_RESOURCE {
    uint32_t version;
    uint32_t subResourceIndex;
    uint32_t inputResource;
    NV_ENC_REGISTERED_PTR registeredResource;
    NV_ENC_INPUT_PTR mappedResource;
    uint32_t mappedBufferFmt;
    uint32_t reserved[251];
    void*    reserved2[63];
};

struct NV_ENC_CREATE_BITSTREAM_BUFFER {
    uint32_t version;
    uint32_t size;
    uint32_t memoryHeap;
    NV_ENC_OUTPUT_PTR bitstreamBuffer;
    void*    bitstreamBufferPtr;
    uint32_t reserved[58];
    void*    reserved2[64];
};

struct NV_ENC_PIC_PARAMS {
    uint32_t version;
    uint32_t inputWidth;
    uint32_t inputHeight;
    uint32_t inputPitch;
    uint32_t encodePicFlags;
    uint32_t frameIdx;
    uint64_t inputTimeStamp;
    uint64_t inputDuration;
    NV_ENC_INPUT_PTR inputBuffer;
    NV_ENC_OUTPUT_PTR outputBitstream;
    void*    completionEvent;
    uint32_t bufferFmt;
    uint32_t pictureStruct;
    uint32_t pictureType;
    NV_ENC_CODEC_CONFIG codecPicParams;
    int8_t*  qpDeltaMap;      // Per-CTU QP delta values for foveated encoding
    uint32_t qpDeltaMapSize;  // Number of entries in qpDeltaMap
    uint32_t reserved[282];
    void*    reserved2[63];
};

struct NV_ENC_LOCK_BITSTREAM {
    uint32_t version;
    uint32_t doNotWait;
    uint32_t ltrFrame;
    NV_ENC_OUTPUT_PTR outputBitstream;
    uint32_t reserved[4];
    void*    bitstreamBufferPtr;
    uint32_t bitstreamSizeInBytes;
    uint32_t outputTimeStamp;
    uint32_t outputDuration;
    uint32_t reserved1;
    uint32_t pictureType;
    uint32_t pictureStruct;
    uint32_t frameAvgQP;
    uint32_t frameSatd;
    uint32_t ltrFrameIdx;
    uint32_t ltrFrameBitmap;
    uint32_t reserved2[13];
    uint32_t intraMBCount;
    uint32_t interMBCount;
    int32_t  averageMVX;
    int32_t  averageMVY;
    uint32_t reserved3[226];
    void*    reserved4[64];
};

// NVENC function table — filled by NvEncodeAPICreateInstance
struct NV_ENCODE_API_FUNCTION_LIST {
    uint32_t version;
    uint32_t reserved;
    NVENCSTATUS (*nvEncOpenEncodeSessionEx)(NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS*, void**);
    void* reserved1[3]; // nvEncGetEncodeGUIDs etc.
    NVENCSTATUS (*nvEncGetEncodePresetConfigEx)(void*, GUID, GUID, uint32_t, void*);
    void* reserved2[3];
    NVENCSTATUS (*nvEncInitializeEncoder)(void*, NV_ENC_INITIALIZE_PARAMS*);
    NVENCSTATUS (*nvEncCreateInputBuffer)(void*, void*);
    NVENCSTATUS (*nvEncDestroyInputBuffer)(void*, NV_ENC_INPUT_PTR);
    NVENCSTATUS (*nvEncCreateBitstreamBuffer)(void*, NV_ENC_CREATE_BITSTREAM_BUFFER*);
    NVENCSTATUS (*nvEncDestroyBitstreamBuffer)(void*, NV_ENC_OUTPUT_PTR);
    NVENCSTATUS (*nvEncEncodePicture)(void*, NV_ENC_PIC_PARAMS*);
    NVENCSTATUS (*nvEncLockBitstream)(void*, NV_ENC_LOCK_BITSTREAM*);
    NVENCSTATUS (*nvEncUnlockBitstream)(void*, NV_ENC_OUTPUT_PTR);
    void* reserved3[2]; // lock/unlock input buffer
    NVENCSTATUS (*nvEncRegisterResource)(void*, NV_ENC_REGISTER_RESOURCE*);
    NVENCSTATUS (*nvEncUnregisterResource)(void*, NV_ENC_REGISTERED_PTR);
    NVENCSTATUS (*nvEncMapInputResource)(void*, NV_ENC_MAP_INPUT_RESOURCE*);
    NVENCSTATUS (*nvEncUnmapInputResource)(void*, NV_ENC_INPUT_PTR);
    NVENCSTATUS (*nvEncDestroyEncoder)(void*);
    void* reserved4[281];
};

typedef NVENCSTATUS (*PFN_NvEncodeAPICreateInstance)(NV_ENCODE_API_FUNCTION_LIST*);

// ============================================================
// NvencEncoder class
// ============================================================

class NvencEncoder {
public:
    struct Config {
        uint32_t width = 1832;
        uint32_t height = 1920;
        uint32_t fps = 90;
        uint32_t bitrate_bps = 80'000'000;
        bool use_hevc = true;
    };

    NvencEncoder() = default;
    ~NvencEncoder();

    NvencEncoder(const NvencEncoder&) = delete;
    NvencEncoder& operator=(const NvencEncoder&) = delete;

    bool init(ID3D11Device* device, const Config& config);
    void shutdown();

    bool encode(ID3D11Texture2D* srcTexture,
                bool forceIdr,
                std::vector<uint8_t>& outNalData,
                bool& outIsIdr);

    void requestIdr();
    bool isInitialized() const { return m_initialized; }
    bool isRealEncoder() const { return m_encoder != nullptr; }
    ID3D11Device* getDevice() const { return m_device.Get(); }

    /// Update gaze position for foveated encoding.
    /// Coordinates are normalized (0-1). Called from tracking data receiver.
    void setGaze(float gazeX, float gazeY, bool valid);

    /// Enable/disable foveated encoding.
    void setFoveatedEnabled(bool enabled) { m_foveatedEnabled = enabled; }

private:
    // NVENC session
    void* m_encoder = nullptr;

    // D3D11 resources
    ComPtr<ID3D11Device> m_device;
    ComPtr<ID3D11DeviceContext> m_context;
    ComPtr<ID3D11Texture2D> m_inputTexture; // Registered with NVENC

    // NVENC resources
    NV_ENCODE_API_FUNCTION_LIST m_nvencFns{};
    void* m_nvencLib = nullptr;
    NV_ENC_REGISTERED_PTR m_registeredResource = nullptr;
    NV_ENC_OUTPUT_PTR m_bitstreamBuffer = nullptr;

    // Encoder state
    Config m_config;
    bool m_initialized = false;
    uint32_t m_frameCount = 0;
    uint32_t m_idrInterval = 180;

    // Foveated encoding state
    bool m_foveatedEnabled = false;
    std::atomic<float> m_gazeX{0.5f};
    std::atomic<float> m_gazeY{0.5f};
    std::atomic<bool> m_gazeValid{false};
    std::vector<int8_t> m_qpDeltaMap; // Per-CTU QP delta map for foveated encoding
    uint32_t m_ctuCols = 0;
    uint32_t m_ctuRows = 0;

    void computeQpDeltaMap(float gazeX, float gazeY);

    bool loadNvencApi();
    bool createEncoderSession();
    bool createResources();
    void generateTestPattern(bool isIdr, std::vector<uint8_t>& outNalData);
};
