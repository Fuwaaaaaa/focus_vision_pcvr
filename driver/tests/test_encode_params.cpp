#include <gtest/gtest.h>
#include "../src/encode_params.h"

// Hardware-independent encode-parameter helpers. The NVENC bitrate comes from
// the engine (FvpConfig.bitrate_bps = `[video] bitrate_mbps`), like the
// encoded dimensions (FvpConfig.encoded_*), so the driver never derives its
// own values that could disagree with STREAM_CONFIG.

TEST(EncodeParams, UsesTheEngineBitrate) {
    // REGRESSION: the driver used encoded_w * encoded_h * 2, i.e. ~7 Mbps at
    // 1832x1920, while bitrate_mbps (sent to the client) said 80.
    EXPECT_EQ(fvp_encode::targetBitrateBps(80'000'000u), 80'000'000u);
    EXPECT_EQ(fvp_encode::targetBitrateBps(200'000'000u), 200'000'000u);
    EXPECT_NE(fvp_encode::targetBitrateBps(80'000'000u), 1832u * 1920u * 2u);
}

TEST(EncodeParams, MissingBitrateFallsBackToDefault) {
    EXPECT_EQ(fvp_encode::targetBitrateBps(0), fvp_encode::kDefaultBitrateBps);
    EXPECT_EQ(fvp_encode::kDefaultBitrateBps, 80'000'000u);
}
