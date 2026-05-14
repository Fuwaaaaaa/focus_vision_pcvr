// VUI (Video Usability Information) configuration tests.
//
// Verifies that `applyVuiFromConfig` writes the correct bytes for the
// color-range and BT.709 signaling we rely on. The function is shared
// between the H.264 and HEVC code paths in `NvencEncoder::init`, so
// breaking it would silently shift every encoded stream's color space —
// these tests catch that at compile-time-feedback-loop speed instead of
// requiring a real GPU + decoder round-trip.

#include <gtest/gtest.h>
#include "../src/nvenc_encoder.h"

namespace {

// Zero-initialized VUI struct: VUI fields are u32 and unspecified after
// allocation, so we start from {} to make the diff between "didn't touch"
// and "touched and set" unambiguous in assertions.
NV_ENC_CONFIG_HEVC_VUI freshVui() {
    NV_ENC_CONFIG_HEVC_VUI vui{};
    return vui;
}

} // namespace

TEST(NvencVui, FullRangeTrueSetsVideoFullRangeFlag) {
    auto vui = freshVui();
    applyVuiFromConfig(vui, /*full_range=*/true);
    EXPECT_EQ(vui.videoFullRangeFlag, 1u);
}

TEST(NvencVui, FullRangeFalseSetsLimitedRange) {
    auto vui = freshVui();
    applyVuiFromConfig(vui, /*full_range=*/false);
    EXPECT_EQ(vui.videoFullRangeFlag, 0u);
}

TEST(NvencVui, AlwaysSignalsVideoTypePresent) {
    // Without videoSignalTypePresentFlag = 1, the decoder ignores the
    // colour-range bit entirely, so this must be set regardless of
    // full_range value.
    auto vui = freshVui();
    applyVuiFromConfig(vui, true);
    EXPECT_EQ(vui.videoSignalTypePresentFlag, 1u);

    auto vui2 = freshVui();
    applyVuiFromConfig(vui2, false);
    EXPECT_EQ(vui2.videoSignalTypePresentFlag, 1u);
}

TEST(NvencVui, EmitsBt709ColorMetadata) {
    // BT.709 (1/1/1 for primaries/transfer/matrix) is the only profile
    // every consumer-grade VR HMD decoder accepts without color shift.
    // Flagging any other value here would surface the regression before
    // a user complains about washed-out reds.
    auto vui = freshVui();
    applyVuiFromConfig(vui, true);
    EXPECT_EQ(vui.colourDescriptionPresentFlag, 1u);
    EXPECT_EQ(vui.colourPrimaries, 1u);
    EXPECT_EQ(vui.transferCharacteristics, 1u);
    EXPECT_EQ(vui.matrixCoeffs, 1u);
}

TEST(NvencVui, VideoFormatUnspecified) {
    // 5 = "Unspecified" in H.264/HEVC VUI semantics. Anything else
    // (PAL=1, NTSC=2, etc.) would lie to the decoder about the source.
    auto vui = freshVui();
    applyVuiFromConfig(vui, true);
    EXPECT_EQ(vui.videoFormat, 5u);
}

TEST(NvencVui, H264AliasReceivesSameWiring) {
    // NV_ENC_CONFIG_H264_VUI is currently `using NV_ENC_CONFIG_HEVC_VUI`,
    // but the template apply is what guarantees both paths stay aligned
    // if those types ever diverge. Pin the behavior so the alias
    // contract is explicit.
    NV_ENC_CONFIG_H264_VUI vui{};
    applyVuiFromConfig(vui, /*full_range=*/true);
    EXPECT_EQ(vui.videoFullRangeFlag, 1u);
    EXPECT_EQ(vui.colourPrimaries, 1u);
}
