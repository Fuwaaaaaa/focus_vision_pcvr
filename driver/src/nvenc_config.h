#pragma once

// Encoder settings on top of NVIDIA's preset, kept free of D3D11 and of the
// NVENC runtime so the driver tests can check them without a GPU.
//
// All types and constants come from the official nvEncodeAPI.h
// (third_party/nvenc, SDK 12.2). The driver used to mirror them by hand and
// got the struct versions, the function table order, several struct layouts,
// NV_ENC_BUFFER_FORMAT_ARGB and NV_ENC_PIC_FLAG_FORCEIDR wrong, so every
// NVENC call would have failed or read the wrong fields.

#include "nvEncodeAPI.h"

#include <cstdint>

namespace fvp_nvenc {

/// Side of the block one qpDeltaMap entry covers: a CTB for HEVC (NVENC
/// supports only 32x32 CTBs) and a macroblock for H.264.
constexpr uint32_t qpMapBlockSize(bool hevc) { return hevc ? 32 : 16; }

/// encodePicFlags for one frame. An IDR also repeats the parameter sets
/// (VPS/SPS/PPS) so a decoder that lost them can restart from it.
constexpr uint32_t picFlagsFor(bool idr) {
    return idr ? (NV_ENC_PIC_FLAG_FORCEIDR | NV_ENC_PIC_FLAG_OUTPUT_SPSPPS) : 0u;
}

/// Colour signalling written into the bitstream: BT.709, full or limited
/// range. H.264 and HEVC share the VUI struct.
template <class Vui>
inline void applyVuiFromConfig(Vui& vui, bool full_range) {
    vui.videoSignalTypePresentFlag = 1;
    vui.videoFormat = NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED;
    vui.videoFullRangeFlag = full_range ? 1 : 0;
    vui.colourDescriptionPresentFlag = 1;
    vui.colourPrimaries = NV_ENC_VUI_COLOR_PRIMARIES_BT709;
    vui.transferCharacteristics = NV_ENC_VUI_TRANSFER_CHARACTERISTIC_BT709;
    vui.colourMatrix = NV_ENC_VUI_MATRIX_COEFFS_BT709;
}

struct StreamSettings {
    bool hevc = true;
    uint32_t bitrate_bps = 80'000'000;
    uint32_t fps = 90;
    bool full_range = true;
    bool qp_delta_map = false;  // foveated encoding
};

/// Low-latency streaming settings on top of the preset config returned by
/// nvEncGetEncodePresetConfigEx:
/// - CBR at the target bitrate, with a one-frame VBV so a frame never waits
///   for buffer room.
/// - IPP only, and no automatic IDRs: NvencEncoder forces them (periodically
///   and when the HMD asks), each with its parameter sets.
/// - HEVC CTBs fixed at 32x32 so the QP delta map matches qpMapBlockSize.
inline void applyStreamSettings(NV_ENC_CONFIG& cfg, const StreamSettings& s) {
    cfg.version = NV_ENC_CONFIG_VER;
    cfg.gopLength = NVENC_INFINITE_GOPLENGTH;
    cfg.frameIntervalP = 1;

    cfg.rcParams.rateControlMode = NV_ENC_PARAMS_RC_CBR;
    cfg.rcParams.averageBitRate = s.bitrate_bps;
    cfg.rcParams.maxBitRate = s.bitrate_bps;
    const uint32_t fps = s.fps != 0 ? s.fps : 90;
    cfg.rcParams.vbvBufferSize = s.bitrate_bps / fps;
    cfg.rcParams.vbvInitialDelay = cfg.rcParams.vbvBufferSize;
    cfg.rcParams.qpMapMode = s.qp_delta_map ? NV_ENC_QP_MAP_DELTA : NV_ENC_QP_MAP_DISABLED;

    if (s.hevc) {
        auto& hevc = cfg.encodeCodecConfig.hevcConfig;
        hevc.idrPeriod = NVENC_INFINITE_GOPLENGTH;
        hevc.repeatSPSPPS = 1;
        hevc.maxCUSize = NV_ENC_HEVC_CUSIZE_32x32;
        applyVuiFromConfig(hevc.hevcVUIParameters, s.full_range);
    } else {
        auto& h264 = cfg.encodeCodecConfig.h264Config;
        h264.idrPeriod = NVENC_INFINITE_GOPLENGTH;
        h264.repeatSPSPPS = 1;
        applyVuiFromConfig(h264.h264VUIParameters, s.full_range);
    }
}

/// NVENC API version the header was written for, in the format
/// NvEncodeAPIGetMaxSupportedVersion reports ((major << 4) | minor).
constexpr uint32_t kRequiredApiVersion = (NVENCAPI_MAJOR_VERSION << 4) | NVENCAPI_MINOR_VERSION;

/// Whether an installed NVIDIA driver (its max supported NVENC API version)
/// can run this header's structs.
constexpr bool driverSupportsApi(uint32_t maxSupportedVersion) {
    return maxSupportedVersion >= kRequiredApiVersion;
}

}  // namespace fvp_nvenc
