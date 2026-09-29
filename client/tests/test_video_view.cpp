// What each eye shows of a decoded frame, and the reprojection applied to an
// older frame — the math renderer.cpp's fragment shader runs.
#include <gtest/gtest.h>

#include <cmath>

#include "video_view.h"

using namespace fvp_video;

namespace {
constexpr float kPi = 3.14159265358979f;
constexpr float kDeg = kPi / 180.0f;

Quat yaw(float degrees) {  // about +Y (up); positive turns left
    return {0.0f, std::sin(degrees * kDeg / 2), 0.0f, std::cos(degrees * kDeg / 2)};
}
Quat pitch(float degrees) {  // about +X; positive tilts up
    return {std::sin(degrees * kDeg / 2), 0.0f, 0.0f, std::cos(degrees * kDeg / 2)};
}
const Quat kLevel{0, 0, 0, 1};
const Tangents kSymmetric = tangents(-50 * kDeg, 50 * kDeg, -50 * kDeg, 50 * kDeg);
}  // namespace

TEST(VideoView, EachEyeShowsItsHalfOfASideBySideFrame) {
    // REGRESSION (mono): the one image went to both eyes.
    const UvRect left = eyeRect(fvp_client_protocol::STEREO_SIDE_BY_SIDE, 0);
    const UvRect right = eyeRect(fvp_client_protocol::STEREO_SIDE_BY_SIDE, 1);
    EXPECT_FLOAT_EQ(left.u0, 0.0f);
    EXPECT_FLOAT_EQ(left.u1, 0.5f);
    EXPECT_FLOAT_EQ(right.u0, 0.5f);
    EXPECT_FLOAT_EQ(right.u1, 1.0f);
    EXPECT_FLOAT_EQ(left.v1, 1.0f);

    const UvRect mono = eyeRect(fvp_client_protocol::STEREO_MONO, 1);
    EXPECT_FLOAT_EQ(mono.u0, 0.0f);
    EXPECT_FLOAT_EQ(mono.u1, 1.0f);

    float fu = 0, fv = 0;
    frameUv(right, 0.5f, 0.25f, fu, fv);
    EXPECT_FLOAT_EQ(fu, 0.75f);
    EXPECT_FLOAT_EQ(fv, 0.25f);
}

TEST(VideoView, ANewFrameFillsTheViewTopDown) {
    // Screen top shows the image's top (v = 0) — on both the new-frame path
    // and the reprojection path. REGRESSION: the timewarp path drew the
    // image upside down relative to the new-frame path.
    float u = 0, v = 0;
    ASSERT_TRUE(sampleUv(-1.0f, 1.0f, kIdentity, kSymmetric, u, v));
    EXPECT_NEAR(u, 0.0f, 1e-5f);
    EXPECT_NEAR(v, 0.0f, 1e-5f);
    ASSERT_TRUE(sampleUv(1.0f, -1.0f, kIdentity, kSymmetric, u, v));
    EXPECT_NEAR(u, 1.0f, 1e-5f);
    EXPECT_NEAR(v, 1.0f, 1e-5f);
    ASSERT_TRUE(sampleUv(0.0f, 0.0f, kIdentity, kSymmetric, u, v));
    EXPECT_NEAR(u, 0.5f, 1e-5f);
    EXPECT_NEAR(v, 0.5f, 1e-5f);
}

TEST(VideoView, AnAsymmetricFieldOfViewKeepsItsCentre) {
    // The straight-ahead direction sits off-centre in an asymmetric FOV.
    const Tangents t = tangents(-55 * kDeg, 45 * kDeg, -50 * kDeg, 40 * kDeg);
    float u = 0, v = 0;
    const float ndcX = (0.0f - t.left) / (t.right - t.left) * 2 - 1;
    const float ndcY = (0.0f - t.down) / (t.up - t.down) * 2 - 1;
    ASSERT_TRUE(sampleUv(ndcX, ndcY, kIdentity, t, u, v));
    EXPECT_NEAR(u, (0.0f - t.left) / (t.right - t.left), 1e-5f);
    EXPECT_NEAR(v, 1.0f - (0.0f - t.down) / (t.up - t.down), 1e-5f);
}

TEST(VideoView, TurningRightShowsWhatWasToTheRight) {
    // The head turned 10° right since the frame: the view's centre now
    // looks at what was 10° right of the frame's centre.
    float rot[9];
    reprojectionRotation(kLevel, yaw(-10), rot);
    float u = 0, v = 0;
    ASSERT_TRUE(sampleUv(0.0f, 0.0f, rot, kSymmetric, u, v));
    EXPECT_NEAR(u, (std::tan(10 * kDeg) - kSymmetric.left) / (kSymmetric.right - kSymmetric.left), 1e-4f);
    EXPECT_GT(u, 0.5f);
    EXPECT_NEAR(v, 0.5f, 1e-5f);
}

TEST(VideoView, TiltingUpShowsWhatWasAbove) {
    float rot[9];
    reprojectionRotation(kLevel, pitch(8), rot);
    float u = 0, v = 0;
    ASSERT_TRUE(sampleUv(0.0f, 0.0f, rot, kSymmetric, u, v));
    EXPECT_NEAR(u, 0.5f, 1e-5f);
    EXPECT_LT(v, 0.5f) << "above = towards the image's top";
    EXPECT_NEAR(v, 1.0f - (std::tan(8 * kDeg) - kSymmetric.down) / (kSymmetric.up - kSymmetric.down), 1e-4f);
}

TEST(VideoView, OnlyTheChangeSinceTheFrameCounts) {
    // Frame drawn facing 30° left, the head now at 20° left: 10° to the
    // right of the frame, as from level to 10° right.
    float fromTurned[9], fromLevel[9];
    reprojectionRotation(yaw(30), yaw(20), fromTurned);
    reprojectionRotation(kLevel, yaw(-10), fromLevel);
    for (int i = 0; i < 9; i++) EXPECT_NEAR(fromTurned[i], fromLevel[i], 1e-5f) << i;
}

TEST(VideoView, DirectionsOutsideTheFrameAreBlack) {
    float rot[9];
    reprojectionRotation(kLevel, yaw(-45), rot);  // 45° right: the right edge is past the frame
    float u = 0, v = 0;
    EXPECT_FALSE(sampleUv(1.0f, 0.0f, rot, kSymmetric, u, v));
    reprojectionRotation(kLevel, yaw(180), rot);  // facing backwards
    EXPECT_FALSE(sampleUv(0.0f, 0.0f, rot, kSymmetric, u, v));
}
