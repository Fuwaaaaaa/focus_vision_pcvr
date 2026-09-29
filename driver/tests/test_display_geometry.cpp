#include <gtest/gtest.h>

#include "display_geometry.h"
#include "gpu_adapter.h"

using namespace fvp_display;
using fvp_gpu::AdapterInfo;
using fvp_gpu::chooseAdapter;

TEST(DisplayGeometry, ProjectionRawIsHalfAngleTangentsWithYDown) {
    const ProjectionRaw p = projectionRaw(Fov{-45.0f, 45.0f, 45.0f, -45.0f});
    EXPECT_NEAR(p.left, -1.0f, 1e-5f);
    EXPECT_NEAR(p.right, 1.0f, 1e-5f);
    EXPECT_NEAR(p.top, -1.0f, 1e-5f) << "OpenVR's raw projection has y down: top is negative";
    EXPECT_NEAR(p.bottom, 1.0f, 1e-5f);
}

TEST(DisplayGeometry, ProjectionRawKeepsAsymmetry) {
    const ProjectionRaw p = projectionRaw(Fov{-50.0f, 40.0f, 30.0f, -60.0f});
    EXPECT_LT(p.left, 0.0f);
    EXPECT_GT(-p.left, p.right) << "wider to the left";
    EXPECT_GT(p.bottom, -p.top) << "wider downwards";
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
