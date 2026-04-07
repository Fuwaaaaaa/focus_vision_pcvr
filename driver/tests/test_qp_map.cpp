#include <gtest/gtest.h>
#include "../src/qp_map.h"

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
