#include <gtest/gtest.h>

#include "display_geometry.h"
#include "gpu_adapter.h"

using namespace fvp_display;
using fvp_gpu::AdapterInfo;
using fvp_gpu::chooseAdapter;

namespace {
// OpenVR's ComposeProjection (the projection SteamVR builds from the raw
// values): NDC y of a direction whose vertical tangent is `t` (y up, looking
// down -Z).
float composedNdcY(const ProjectionRaw& p, float t) {
    const float idy = 1.0f / (p.bottom - p.top);
    const float sy = p.bottom + p.top;
    // clip.y = 2*idy*t + sy*idy*z with z = -1; w = 1.
    return 2 * idy * t - sy * idy;
}
float composedNdcX(const ProjectionRaw& p, float t) {
    const float idx = 1.0f / (p.right - p.left);
    const float sx = p.right + p.left;
    return 2 * idx * t - sx * idx;
}
constexpr float kDeg = 3.14159265358979f / 180.0f;
}  // namespace

TEST(DisplayGeometry, ProjectionRawIsHalfAngleTangents) {
    const ProjectionRaw p = projectionRaw(Fov{-45.0f, 45.0f, 45.0f, -45.0f});
    EXPECT_NEAR(p.left, -1.0f, 1e-5f);
    EXPECT_NEAR(p.right, 1.0f, 1e-5f);
    EXPECT_NEAR(p.top, -1.0f, 1e-5f);
    EXPECT_NEAR(p.bottom, 1.0f, 1e-5f);
}

TEST(DisplayGeometry, TheImageEdgesAreTheFieldOfViewEdges) {
    // REGRESSION: top and bottom were -tan(up) and -tan(down), which puts an
    // asymmetric field of view upside down in OpenVR's projection. Wider
    // downwards (as headsets are): up 30°, down 60°.
    const Fov fov{-50.0f, 40.0f, 30.0f, -60.0f};
    const ProjectionRaw p = projectionRaw(fov);
    EXPECT_NEAR(composedNdcY(p, std::tan(fov.up * kDeg)), 1.0f, 1e-5f) << "the up edge at the image's top";
    EXPECT_NEAR(composedNdcY(p, std::tan(fov.down * kDeg)), -1.0f, 1e-5f) << "the down edge at its bottom";
    EXPECT_NEAR(composedNdcX(p, std::tan(fov.left * kDeg)), -1.0f, 1e-5f);
    EXPECT_NEAR(composedNdcX(p, std::tan(fov.right * kDeg)), 1.0f, 1e-5f);
    EXPECT_NEAR(composedNdcY(p, 0.0f), 0.5f, 1e-5f) << "straight ahead sits above centre when there is more below";
}

TEST(DisplayGeometry, HeadsetRadiansBecomeDegrees) {
    const Fov fov = fovFromRadians(-52 * kDeg, 45 * kDeg, 41 * kDeg, -49 * kDeg);
    EXPECT_NEAR(fov.left, -52.0f, 1e-4f);
    EXPECT_NEAR(fov.right, 45.0f, 1e-4f);
    EXPECT_NEAR(fov.up, 41.0f, 1e-4f);
    EXPECT_NEAR(fov.down, -49.0f, 1e-4f);
}

TEST(DisplayGeometry, EyesSitHalfTheIpdEitherSideOfTheHead) {
    const EyeToHead left = eyeToHead(-0.032f);
    const EyeToHead right = eyeToHead(0.032f);
    EXPECT_FLOAT_EQ(left.m[0][3], -0.032f) << "translation in the last column";
    EXPECT_FLOAT_EQ(right.m[0][3], 0.032f);
    EXPECT_FLOAT_EQ(right.m[1][3], 0.0f);
    EXPECT_FLOAT_EQ(right.m[2][3], 0.0f);
    for (int r = 0; r < 3; r++)
        for (int c = 0; c < 3; c++) EXPECT_FLOAT_EQ(right.m[r][c], r == c ? 1.0f : 0.0f) << "no rotation";
}

TEST(DisplayGeometry, EyesSitSideBySide) {
    const Viewport left = eyeViewport(0, 1832, 1920);
    const Viewport right = eyeViewport(1, 1832, 1920);
    EXPECT_EQ(left.x, 0u);
    EXPECT_EQ(right.x, 1832u);
    EXPECT_EQ(left.y, 0u);
    EXPECT_EQ(right.width, 1832u);
    EXPECT_EQ(right.height, 1920u);
}

TEST(GpuAdapter, PrefersNvidiaOverALargerGpu) {
    // NVENC needs the NVIDIA GPU even when another has more memory.
    const std::vector<AdapterInfo> adapters = {
        {0x1002, 1, 16ull << 30, false},  // AMD, 16 GB
        {fvp_gpu::kNvidiaVendorId, 2, 8ull << 30, false},
        {0x8086, 3, 0, false},  // Intel iGPU
    };
    EXPECT_EQ(chooseAdapter(adapters), 1);
}

TEST(GpuAdapter, PicksTheNvidiaWithMostMemory) {
    const std::vector<AdapterInfo> adapters = {
        {fvp_gpu::kNvidiaVendorId, 1, 6ull << 30, false},
        {fvp_gpu::kNvidiaVendorId, 2, 12ull << 30, false},
    };
    EXPECT_EQ(chooseAdapter(adapters), 1);
}

TEST(GpuAdapter, FallsBackToTheLargestHardwareGpuAndSkipsSoftware) {
    const std::vector<AdapterInfo> adapters = {
        {0x1414, 1, 0, true},  // Microsoft Basic Render Driver
        {0x8086, 2, 128ull << 20, false},
        {0x1002, 3, 8ull << 30, false},
    };
    EXPECT_EQ(chooseAdapter(adapters), 2);
}

TEST(GpuAdapter, NoneWithOnlySoftwareAdapters) {
    EXPECT_EQ(chooseAdapter({{0x1414, 1, 0, true}}), -1);
    EXPECT_EQ(chooseAdapter({}), -1);
}

TEST(GpuAdapter, LuidPacksLowPartThenHighPart) {
    // The LUID struct's bytes (LowPart, then HighPart) read as a uint64.
    EXPECT_EQ(fvp_gpu::packLuid(0x89abcdefu, 0x01234567), 0x0123456789abcdefull);
    EXPECT_EQ(fvp_gpu::packLuid(1u, -1), 0xffffffff00000001ull);
}
