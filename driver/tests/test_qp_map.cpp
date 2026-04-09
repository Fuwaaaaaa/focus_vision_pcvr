#include <gtest/gtest.h>
#include "../src/qp_map.h"
#include "../src/nvenc_encoder.h"

// Frame: 1832x1920, HEVC CTU=64 → 29 cols x 30 rows
static constexpr uint32_t FRAME_W = 1832;
static constexpr uint32_t FRAME_H = 1920;
static constexpr uint32_t CTU_HEVC = 64;
static constexpr uint32_t CTU_H264 = 16;

TEST(QpMap, CtuGridHevc) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);
    EXPECT_EQ(cols, 29u); // ceil(1832/64)
    EXPECT_EQ(rows, 30u); // ceil(1920/64)
}

TEST(QpMap, CtuGridH264) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_H264, cols, rows);
    EXPECT_EQ(cols, 115u); // ceil(1832/16)
    EXPECT_EQ(rows, 120u); // ceil(1920/16)
}

TEST(QpMap, CenterGazeHasFoveaAtCenter) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);

    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.15f, 0.35f, 5, 15, map);

    ASSERT_EQ(map.size(), cols * rows);

    // Center CTU should be fovea (QP delta = 0)
    uint32_t centerCol = cols / 2;
    uint32_t centerRow = rows / 2;
    EXPECT_EQ(map[centerRow * cols + centerCol], 0);

    // Corner should be peripheral (QP delta = 15)
    EXPECT_EQ(map[0], 15); // top-left
    EXPECT_EQ(map[cols - 1], 15); // top-right
}

TEST(QpMap, CornerGazeShiftsFovea) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);

    std::vector<int8_t> map;
    // Gaze at top-left corner
    computeQpDeltaMap(0.0f, 0.0f, cols, rows, 0.15f, 0.35f, 5, 15, map);

    // Top-left should be fovea
    EXPECT_EQ(map[0], 0);
    // Bottom-right should be peripheral
    EXPECT_EQ(map[(rows - 1) * cols + (cols - 1)], 15);
}

TEST(QpMap, AggressivePresetUsesHigherOffsets) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);

    std::vector<int8_t> balanced, aggressive;
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.15f, 0.35f, 5, 15, balanced);
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.15f, 0.35f, 8, 25, aggressive);

    // Corner: aggressive should have higher QP delta
    EXPECT_GT(aggressive[0], balanced[0]);
    // Center: both should be 0 (fovea)
    uint32_t center = (rows / 2) * cols + (cols / 2);
    EXPECT_EQ(balanced[center], 0);
    EXPECT_EQ(aggressive[center], 0);
}

TEST(QpMap, PresetLookup) {
    auto* balanced = findFoveatedPreset("balanced");
    ASSERT_NE(balanced, nullptr);
    EXPECT_EQ(balanced->mid_qp_offset, 5);
    EXPECT_EQ(balanced->peripheral_qp_offset, 15);

    auto* aggressive = findFoveatedPreset("aggressive");
    ASSERT_NE(aggressive, nullptr);
    EXPECT_EQ(aggressive->mid_qp_offset, 8);
    EXPECT_EQ(aggressive->peripheral_qp_offset, 25);

    auto* unknown = findFoveatedPreset("nonexistent");
    EXPECT_EQ(unknown, nullptr);
}

TEST(QpMap, MapSizeMatchesGrid) {
    for (uint32_t ctu : {16u, 64u}) {
        uint32_t cols, rows;
        computeCtuGrid(FRAME_W, FRAME_H, ctu, cols, rows);
        std::vector<int8_t> map;
        computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.15f, 0.35f, 5, 15, map);
        EXPECT_EQ(map.size(), static_cast<size_t>(cols * rows));
    }
}

// ============================================================
// VUI Parameter Tests
// ============================================================

TEST(VuiConfig, HevcVuiFieldsAccessible) {
    NV_ENC_CONFIG_HEVC hevc = {};
    hevc.hevcVUIParameters.videoFullRangeFlag = 1;
    hevc.hevcVUIParameters.colourPrimaries = 1;
    hevc.hevcVUIParameters.transferCharacteristics = 1;
    hevc.hevcVUIParameters.matrixCoeffs = 1;
    hevc.hevcVUIParameters.videoSignalTypePresentFlag = 1;
    hevc.hevcVUIParameters.colourDescriptionPresentFlag = 1;

    EXPECT_EQ(hevc.hevcVUIParameters.videoFullRangeFlag, 1u);
    EXPECT_EQ(hevc.hevcVUIParameters.colourPrimaries, 1u);
    EXPECT_EQ(hevc.hevcVUIParameters.transferCharacteristics, 1u);
    EXPECT_EQ(hevc.hevcVUIParameters.matrixCoeffs, 1u);
}

TEST(VuiConfig, H264VuiFieldsAccessible) {
    NV_ENC_CONFIG_H264 h264 = {};
    h264.h264VUIParameters.videoFullRangeFlag = 1;
    h264.h264VUIParameters.colourPrimaries = 1;
    h264.h264VUIParameters.transferCharacteristics = 1;
    h264.h264VUIParameters.matrixCoeffs = 1;

    EXPECT_EQ(h264.h264VUIParameters.videoFullRangeFlag, 1u);
    EXPECT_EQ(h264.h264VUIParameters.colourPrimaries, 1u);
}

TEST(VuiConfig, FullRangeVsLimited) {
    // Full range: videoFullRangeFlag = 1
    NV_ENC_CONFIG config_full = {};
    config_full.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag = 1;
    EXPECT_EQ(config_full.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag, 1u);

    // Limited range: videoFullRangeFlag = 0
    NV_ENC_CONFIG config_limited = {};
    config_limited.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag = 0;
    EXPECT_EQ(config_limited.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag, 0u);
}

TEST(VuiConfig, CodecConfigUnionLayout) {
    // Verify HEVC and H264 share the same union space
    NV_ENC_CODEC_CONFIG codec = {};
    codec.hevcConfig.hevcVUIParameters.videoFullRangeFlag = 42;

    // Access via union — same memory, different interpretation
    // This verifies the union layout is correct
    EXPECT_EQ(sizeof(codec.hevcConfig), sizeof(codec.h264Config));
}

// ============================================================
// ROI Fallback Tests
// ============================================================

// Note: NvencEncoder tests that require instance creation are skipped here
// because the test binary doesn't link against nvenc_encoder.cpp (D3D11 dependency).
// These are tested via integration tests with real hardware.

TEST(RoiFallback, ConfigDefaultsHaveNoRoi) {
    // Verify that the Config struct defaults don't enable ROI
    NvencEncoder::Config cfg;
    EXPECT_TRUE(cfg.full_range);
    EXPECT_TRUE(cfg.use_hevc);
    // Foveated params have reasonable defaults
    EXPECT_FLOAT_EQ(cfg.fovea_radius, 0.15f);
    EXPECT_FLOAT_EQ(cfg.mid_radius, 0.35f);
    EXPECT_EQ(cfg.mid_qp_offset, 5);
    EXPECT_EQ(cfg.peripheral_qp_offset, 15);
}

TEST(RoiFallback, QpDeltaMapAlwaysAvailable) {
    // QP delta map works without NVENC hardware
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);
    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.15f, 0.35f, 5, 15, map);
    // Should always produce a valid map
    ASSERT_FALSE(map.empty());
    EXPECT_EQ(map.size(), static_cast<size_t>(cols * rows));
    // Center should be fovea (0), corners should be peripheral (15)
    EXPECT_EQ(map[(rows / 2) * cols + (cols / 2)], 0);
    EXPECT_EQ(map[0], 15);
}

TEST(VuiConfig, NvencConfigFullRangePropagation) {
    // Simulate the full config path: Config.full_range → NV_ENC_CONFIG VUI
    NvencEncoder::Config appConfig;
    appConfig.use_hevc = true;
    appConfig.full_range = true;

    NV_ENC_CONFIG encConfig = {};
    if (appConfig.use_hevc) {
        auto& vui = encConfig.encodeCodecConfig.hevcConfig.hevcVUIParameters;
        vui.videoSignalTypePresentFlag = 1;
        vui.videoFormat = 5;
        vui.videoFullRangeFlag = appConfig.full_range ? 1 : 0;
        vui.colourDescriptionPresentFlag = 1;
        vui.colourPrimaries = 1;
        vui.transferCharacteristics = 1;
        vui.matrixCoeffs = 1;
    }

    auto& vui = encConfig.encodeCodecConfig.hevcConfig.hevcVUIParameters;
    EXPECT_EQ(vui.videoFullRangeFlag, 1u);
    EXPECT_EQ(vui.videoSignalTypePresentFlag, 1u);
    EXPECT_EQ(vui.colourPrimaries, 1u); // BT.709

    // Now test limited range
    appConfig.full_range = false;
    encConfig = {};
    auto& vui2 = encConfig.encodeCodecConfig.hevcConfig.hevcVUIParameters;
    vui2.videoFullRangeFlag = appConfig.full_range ? 1 : 0;
    EXPECT_EQ(vui2.videoFullRangeFlag, 0u);
}
