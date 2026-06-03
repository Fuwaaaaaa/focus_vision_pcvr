#include <gtest/gtest.h>
#include "../src/encode_params.h"

// Hardware-independent encode-parameter math (NVENC bitrate from the encoded
// frame size). Encoded dimensions themselves come from the engine
// (FvpConfig.encoded_*, computed once in Rust) — the driver never recomputes
// them, so PC encode resolution and STREAM_CONFIG can never disagree.

TEST(EncodeParams, BitrateMatchesLegacyFormulaAtFactor2) {
    // Native 1832x1920 with the default factor reproduces width*height*2.
    EXPECT_EQ(fvp_encode::computeBitrateBps(1832, 1920, 2.0f), 1832u * 1920u * 2u);
}

TEST(EncodeParams, BitrateScalesWithEncodedArea) {
    // Half resolution -> a quarter of the pixels -> a quarter of the bitrate.
    uint32_t full = fvp_encode::computeBitrateBps(1832, 1920, 2.0f);
    uint32_t half = fvp_encode::computeBitrateBps(916, 960, 2.0f);
    EXPECT_EQ(half, full / 4);
}

TEST(EncodeParams, BitratePixelFactorRaisesBits) {
    uint32_t f2 = fvp_encode::computeBitrateBps(916, 960, 2.0f);
    uint32_t f3 = fvp_encode::computeBitrateBps(916, 960, 3.0f);
    EXPECT_GT(f3, f2);
    EXPECT_EQ(f3, 916u * 960u * 3u);
}
