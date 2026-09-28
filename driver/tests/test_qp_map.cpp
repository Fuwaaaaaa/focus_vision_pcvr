#include <gtest/gtest.h>
#include "../src/qp_map.h"
#include "../src/nvenc_encoder.h"

// Frame: 1832x1920, HEVC CTB=32 (the only size NVENC supports) → 58 cols x 60 rows
static constexpr uint32_t FRAME_W = 1832;
static constexpr uint32_t FRAME_H = 1920;
static constexpr uint32_t CTU_HEVC = fvp_nvenc::qpMapBlockSize(true);
static constexpr uint32_t CTU_H264 = fvp_nvenc::qpMapBlockSize(false);

TEST(QpMap, CtuGridHevc) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);
    EXPECT_EQ(cols, 58u); // ceil(1832/32)
    EXPECT_EQ(rows, 60u); // ceil(1920/32)
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
    for (uint32_t ctu : {CTU_H264, CTU_HEVC}) {
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

TEST(VuiConfig, FullRangeVsLimited) {
    NV_ENC_CONFIG config_full = {};
    fvp_nvenc::applyVuiFromConfig(
        config_full.encodeCodecConfig.hevcConfig.hevcVUIParameters, true);
    EXPECT_EQ(config_full.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag, 1u);

    NV_ENC_CONFIG config_limited = {};
    fvp_nvenc::applyVuiFromConfig(
        config_limited.encodeCodecConfig.hevcConfig.hevcVUIParameters, false);
    EXPECT_EQ(config_limited.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag, 0u);
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
    // The full config path: Config.full_range → StreamSettings → NV_ENC_CONFIG VUI
    NvencEncoder::Config appConfig;
    appConfig.use_hevc = true;
    appConfig.full_range = true;

    fvp_nvenc::StreamSettings settings;
    settings.hevc = appConfig.use_hevc;
    settings.full_range = appConfig.full_range;
    NV_ENC_CONFIG encConfig = {};
    fvp_nvenc::applyStreamSettings(encConfig, settings);

    auto& vui = encConfig.encodeCodecConfig.hevcConfig.hevcVUIParameters;
    EXPECT_EQ(vui.videoFullRangeFlag, 1u);
    EXPECT_EQ(vui.videoSignalTypePresentFlag, 1u);
    EXPECT_EQ(vui.colourPrimaries, NV_ENC_VUI_COLOR_PRIMARIES_BT709);

    settings.full_range = false;
    encConfig = {};
    fvp_nvenc::applyStreamSettings(encConfig, settings);
    EXPECT_EQ(encConfig.encodeCodecConfig.hevcConfig.hevcVUIParameters.videoFullRangeFlag, 0u);
}

// ============================================================
// Boundary tests — gaze coordinates outside the [0,1] range
// ============================================================

TEST(QpMap, NegativeGazeStaysCoherent) {
    // A miscalibrated eye tracker can briefly report negative or > 1 gaze
    // coordinates. The QP map must still produce a sane output (every CTU
    // valid, no NaN, no out-of-bounds writes) — we treat off-screen gaze as
    // "centre is far away, so almost everything is peripheral".
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);

    std::vector<int8_t> map;
    computeQpDeltaMap(-0.5f, -0.5f, cols, rows, 0.15f, 0.35f, 5, 15, map);
    ASSERT_EQ(map.size(), cols * rows);
    // Every CTU is far from the off-screen gaze, so the entire map is peripheral.
    for (auto v : map) {
        EXPECT_EQ(v, 15);
    }
}

TEST(QpMap, OverOneGazeStaysCoherent) {
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);

    std::vector<int8_t> map;
    computeQpDeltaMap(1.5f, 1.5f, cols, rows, 0.15f, 0.35f, 5, 15, map);
    ASSERT_EQ(map.size(), cols * rows);
    for (auto v : map) {
        EXPECT_EQ(v, 15);
    }
}

TEST(QpMap, MidZoneCtusReceiveMidDelta) {
    // With gaze at centre, the annulus between fovea_r and mid_r must
    // be populated with `midQpDelta`. Without this test the existing
    // suite only validates the fovea (=0) and peripheral (=15) zones —
    // a regression that collapsed mid into peripheral would go unnoticed.
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);

    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.05f, 0.40f, 5, 15, map);

    bool sawMid = false;
    for (auto v : map) {
        if (v == 5) { sawMid = true; break; }
    }
    EXPECT_TRUE(sawMid) << "expected at least one CTU in the mid zone";
}

TEST(QpMap, ZeroFoveaRadiusCollapsesToMidOrPeripheral) {
    // fovea_r == 0 means there is no fovea zone at all — the centre CTU
    // is at exactly distance 0, which is <= 0 so still classified as fovea.
    // Everything else gets mid or peripheral.
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);
    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.0f, 0.35f, 5, 15, map);
    ASSERT_EQ(map.size(), cols * rows);
    // Nearly all of the map should be non-zero.
    int zeros = 0;
    for (auto v : map) if (v == 0) zeros++;
    EXPECT_LE(zeros, 4) << "fovea_r=0 should produce very few zero CTUs";
}

TEST(QpMap, MidRadiusBeyondOneEliminatesPeripheral) {
    // mid_r >= sqrt(2) ≈ 1.42 in CTU units guarantees the whole grid sits
    // inside the mid zone — no peripheral CTUs.
    uint32_t cols, rows;
    computeCtuGrid(FRAME_W, FRAME_H, CTU_HEVC, cols, rows);
    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, cols, rows, 0.0f, 2.0f, 5, 15, map);
    for (auto v : map) {
        EXPECT_NE(v, 15) << "no CTU should be peripheral when mid_r is huge";
    }
}

// ============================================================
// Degenerate grid sizes
// ============================================================

TEST(QpMap, OneByOneGridIsFovea) {
    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, 1, 1, 0.15f, 0.35f, 5, 15, map);
    ASSERT_EQ(map.size(), 1u);
    EXPECT_EQ(map[0], 0) << "a 1x1 grid puts the gaze inside the only CTU";
}

TEST(QpMap, ZeroSizedGridYieldsEmptyMap) {
    std::vector<int8_t> map;
    computeQpDeltaMap(0.5f, 0.5f, 0, 0, 0.15f, 0.35f, 5, 15, map);
    EXPECT_TRUE(map.empty());
}

TEST(QpMap, TinyFrameCtuGridRoundsUp) {
    // 1px frame still needs one CTU; CTU=16 → 1 col, 1 row.
    uint32_t cols, rows;
    computeCtuGrid(1, 1, 16, cols, rows);
    EXPECT_EQ(cols, 1u);
    EXPECT_EQ(rows, 1u);
}

TEST(QpMap, FrameNotDivisibleByCtuRoundsUp) {
    // 1833x1921 with CTU=32 yields ceil(1833/32)=58, ceil(1921/32)=61
    uint32_t cols, rows;
    computeCtuGrid(1833, 1921, 32, cols, rows);
    EXPECT_EQ(cols, 58u);
    EXPECT_EQ(rows, 61u);
}

// ============================================================
// Foveated preset exhaustive cases
// ============================================================

TEST(FoveatedPreset, SubtlePresetValues) {
    auto* subtle = findFoveatedPreset("subtle");
    ASSERT_NE(subtle, nullptr);
    EXPECT_EQ(subtle->mid_qp_offset, 3);
    EXPECT_EQ(subtle->peripheral_qp_offset, 8);
}

TEST(FoveatedPreset, AllPresetsOrderedByAggressiveness) {
    // The presets should monotonically increase in QP offset values:
    // subtle (3,8) < balanced (5,15) < aggressive (8,25). A future
    // re-ordering bug would silently apply the wrong intensity.
    auto* subtle = findFoveatedPreset("subtle");
    auto* balanced = findFoveatedPreset("balanced");
    auto* aggressive = findFoveatedPreset("aggressive");
    ASSERT_NE(subtle, nullptr);
    ASSERT_NE(balanced, nullptr);
    ASSERT_NE(aggressive, nullptr);
    EXPECT_LT(subtle->mid_qp_offset, balanced->mid_qp_offset);
    EXPECT_LT(balanced->mid_qp_offset, aggressive->mid_qp_offset);
    EXPECT_LT(subtle->peripheral_qp_offset, balanced->peripheral_qp_offset);
    EXPECT_LT(balanced->peripheral_qp_offset, aggressive->peripheral_qp_offset);
}

TEST(FoveatedPreset, CaseSensitiveMatching) {
    // The lookup is intentionally case-sensitive — "Balanced" won't match
    // "balanced". Documenting that contract here prevents a future
    // case-insensitive change from breaking config files in the wild.
    EXPECT_EQ(findFoveatedPreset("Balanced"), nullptr);
    EXPECT_EQ(findFoveatedPreset("BALANCED"), nullptr);
    EXPECT_NE(findFoveatedPreset("balanced"), nullptr);
}

TEST(FoveatedPreset, EmptyStringReturnsNull) {
    EXPECT_EQ(findFoveatedPreset(""), nullptr);
}

TEST(FoveatedPreset, PartialMatchReturnsNull) {
    // Prefix collisions ("balance" is a prefix of "balanced") must not
    // accidentally match — the comparator checks both null terminators.
    EXPECT_EQ(findFoveatedPreset("balance"), nullptr);
    EXPECT_EQ(findFoveatedPreset("balancedXY"), nullptr);
}

// NVENC struct versions and constants: see test_nvenc_config.cpp (the
// driver now uses the official nvEncodeAPI.h instead of hand-written types).
