// VUI (Video Usability Information) configuration tests.
//
// Verifies that `applyVuiFromConfig` writes the correct values for the
// color-range and BT.709 signaling we rely on. The function is shared
// between the H.264 and HEVC code paths in `NvencEncoder::init`, so
// breaking it would silently shift every encoded stream's color space —
// these tests catch that at compile-time-feedback-loop speed instead of
// requiring a real GPU + decoder round-trip.

#include <gtest/gtest.h>
#include "../src/nvenc_config.h"

using fvp_nvenc::applyVuiFromConfig;

namespace {

// Zero-initialized VUI struct, so the diff between "didn't touch" and
// "touched and set" is unambiguous in assertions.
NV_ENC_CONFIG_HEVC_VUI_PARAMETERS freshVui() {
    NV_ENC_CONFIG_HEVC_VUI_PARAMETERS vui{};
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
    EXPECT_EQ(vui.colourPrimaries, NV_ENC_VUI_COLOR_PRIMARIES_BT709);
    EXPECT_EQ(vui.transferCharacteristics, NV_ENC_VUI_TRANSFER_CHARACTERISTIC_BT709);
    EXPECT_EQ(vui.colourMatrix, NV_ENC_VUI_MATRIX_COEFFS_BT709);
    EXPECT_EQ(static_cast<int>(NV_ENC_VUI_COLOR_PRIMARIES_BT709), 1);
}

TEST(NvencVui, VideoFormatUnspecified) {
    // 5 = "Unspecified" in H.264/HEVC VUI semantics. Anything else
    // (PAL=1, NTSC=2, etc.) would lie to the decoder about the source.
    auto vui = freshVui();
    applyVuiFromConfig(vui, true);
    EXPECT_EQ(vui.videoFormat, NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED);
    EXPECT_EQ(static_cast<int>(NV_ENC_VUI_VIDEO_FORMAT_UNSPECIFIED), 5);
}

TEST(NvencVui, H264VuiReceivesSameWiring) {
    // The SDK defines the HEVC VUI type as a typedef of the H.264 one; the
    // template keeps both paths aligned if that ever changes.
    NV_ENC_CONFIG_H264_VUI_PARAMETERS vui{};
    applyVuiFromConfig(vui, /*full_range=*/true);
    EXPECT_EQ(vui.videoFullRangeFlag, 1u);
    EXPECT_EQ(vui.colourPrimaries, NV_ENC_VUI_COLOR_PRIMARIES_BT709);
}
