// Encoder settings built on top of the NVENC preset (nvenc_config.h).
//
// The driver used to hand-mirror nvEncodeAPI.h and got the struct versions,
// the function table order, struct layouts, the ARGB buffer format and the
// FORCEIDR flag wrong. It now uses the official header; these tests pin the
// values the encoder depends on and the settings it applies, without a GPU.

#include <gtest/gtest.h>
#include "../src/nvenc_config.h"

#include <cstddef>

using namespace fvp_nvenc;

TEST(NvencApi, OfficialConstantsTheEncoderReliesOn) {
    // REGRESSION: the hand-written header had ARGB = 0x20 and FORCEIDR = 4
    // (4 is OUTPUT_SPSPPS), so IDR requests never produced an IDR.
    EXPECT_EQ(static_cast<uint32_t>(NV_ENC_BUFFER_FORMAT_ARGB), 0x01000000u);
    EXPECT_EQ(static_cast<uint32_t>(NV_ENC_PIC_FLAG_FORCEIDR), 0x2u);
    EXPECT_EQ(static_cast<uint32_t>(NV_ENC_PIC_FLAG_OUTPUT_SPSPPS), 0x4u);
    EXPECT_EQ(NVENCAPI_MAJOR_VERSION, 12);
    EXPECT_EQ(NVENCAPI_MINOR_VERSION, 2);
}

TEST(NvencApi, StructVersionsCarryTheApiVersionNotTheSize) {
    // REGRESSION: versions were built as sizeof | ver << 16 | API << 24,
    // which NvEncodeAPICreateInstance rejects with NV_ENC_ERR_INVALID_VERSION.
    EXPECT_EQ(NV_ENCODE_API_FUNCTION_LIST_VER & 0xFFFFu, static_cast<uint32_t>(NVENCAPI_VERSION) & 0xFFFFu);
    EXPECT_EQ(NV_ENCODE_API_FUNCTION_LIST_VER >> 28, 0x7u);
    EXPECT_NE(NV_ENC_CONFIG_VER & (1u << 31), 0u);
}

TEST(NvencApi, FunctionTableStartsWithTheLegacyOpenSession) {
    // REGRESSION: the hand-written table put nvEncOpenEncodeSessionEx first,
    // so every later slot pointed at a different function.
    EXPECT_EQ(offsetof(NV_ENCODE_API_FUNCTION_LIST, nvEncOpenEncodeSession), 8u);
    EXPECT_GT(offsetof(NV_ENCODE_API_FUNCTION_LIST, nvEncOpenEncodeSessionEx),
              offsetof(NV_ENCODE_API_FUNCTION_LIST, nvEncDestroyEncoder));
}

TEST(NvencConfig, IdrFramesAreForcedAndCarryParameterSets) {
    EXPECT_EQ(picFlagsFor(false), 0u);
    const uint32_t idr = picFlagsFor(true);
    EXPECT_NE(idr & NV_ENC_PIC_FLAG_FORCEIDR, 0u);
    EXPECT_NE(idr & NV_ENC_PIC_FLAG_OUTPUT_SPSPPS, 0u);
}

TEST(NvencConfig, QpMapBlockMatchesTheCodec) {
    // NVENC supports only 32x32 HEVC CTBs (the map used to assume 64).
    EXPECT_EQ(qpMapBlockSize(true), 32u);
    EXPECT_EQ(qpMapBlockSize(false), 16u);
}

TEST(NvencConfig, LowLatencyCbrOnTopOfThePreset) {
    NV_ENC_CONFIG cfg{};
    cfg.rcParams.rateControlMode = NV_ENC_PARAMS_RC_VBR;  // whatever the preset chose
    StreamSettings s;
    s.bitrate_bps = 90'000'000;
    s.fps = 90;
    applyStreamSettings(cfg, s);

    EXPECT_EQ(cfg.version, static_cast<uint32_t>(NV_ENC_CONFIG_VER));
    EXPECT_EQ(cfg.rcParams.rateControlMode, NV_ENC_PARAMS_RC_CBR);
    EXPECT_EQ(cfg.rcParams.averageBitRate, 90'000'000u);
    EXPECT_EQ(cfg.rcParams.maxBitRate, 90'000'000u);
    EXPECT_EQ(cfg.rcParams.vbvBufferSize, 1'000'000u) << "one frame of bits";
    EXPECT_EQ(cfg.rcParams.vbvInitialDelay, cfg.rcParams.vbvBufferSize);
    EXPECT_EQ(cfg.frameIntervalP, 1) << "no B-frames";
    EXPECT_EQ(cfg.gopLength, static_cast<uint32_t>(NVENC_INFINITE_GOPLENGTH));
}

TEST(NvencConfig, HevcSettings) {
    NV_ENC_CONFIG cfg{};
    StreamSettings s;
    s.hevc = true;
    s.full_range = false;
    applyStreamSettings(cfg, s);

    const auto& hevc = cfg.encodeCodecConfig.hevcConfig;
    EXPECT_EQ(hevc.idrPeriod, static_cast<uint32_t>(NVENC_INFINITE_GOPLENGTH));
    EXPECT_EQ(hevc.repeatSPSPPS, 1u);
    EXPECT_EQ(hevc.maxCUSize, NV_ENC_HEVC_CUSIZE_32x32);
    EXPECT_EQ(hevc.hevcVUIParameters.videoFullRangeFlag, 0u);
    EXPECT_EQ(hevc.hevcVUIParameters.colourMatrix, NV_ENC_VUI_MATRIX_COEFFS_BT709);
}

TEST(NvencConfig, H264Settings) {
    NV_ENC_CONFIG cfg{};
    StreamSettings s;
    s.hevc = false;
    applyStreamSettings(cfg, s);

    const auto& h264 = cfg.encodeCodecConfig.h264Config;
    EXPECT_EQ(h264.idrPeriod, static_cast<uint32_t>(NVENC_INFINITE_GOPLENGTH));
    EXPECT_EQ(h264.repeatSPSPPS, 1u);
    EXPECT_EQ(h264.h264VUIParameters.videoFullRangeFlag, 1u);
}

TEST(NvencConfig, QpDeltaMapModeFollowsFoveation) {
    NV_ENC_CONFIG cfg{};
    StreamSettings s;
    s.qp_delta_map = true;
    applyStreamSettings(cfg, s);
    EXPECT_EQ(cfg.rcParams.qpMapMode, NV_ENC_QP_MAP_DELTA);

    s.qp_delta_map = false;
    applyStreamSettings(cfg, s);
    EXPECT_EQ(cfg.rcParams.qpMapMode, NV_ENC_QP_MAP_DISABLED);
}

TEST(NvencConfig, ZeroFpsDoesNotDivideByZero) {
    NV_ENC_CONFIG cfg{};
    StreamSettings s;
    s.fps = 0;
    s.bitrate_bps = 90'000'000;
    applyStreamSettings(cfg, s);
    EXPECT_EQ(cfg.rcParams.vbvBufferSize, 1'000'000u);
}

TEST(NvencConfig, DriverVersionGate) {
    EXPECT_TRUE(driverSupportsApi((12u << 4) | 2u));
    EXPECT_TRUE(driverSupportsApi((13u << 4) | 0u));
    EXPECT_FALSE(driverSupportsApi((12u << 4) | 1u));
    EXPECT_FALSE(driverSupportsApi((11u << 4) | 1u));
    EXPECT_FALSE(driverSupportsApi(0));
}
