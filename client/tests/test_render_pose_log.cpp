#include <gtest/gtest.h>

#include "render_pose_log.h"

namespace {

fvp_client_protocol::FramePose pose(float y) {
    fvp_client_protocol::FramePose p;
    p.known = true;
    p.orientation[1] = y;
    p.orientation[3] = 1.0f;
    return p;
}

}  // namespace

TEST(RenderPoseLog, AFrameIsFoundByItsIndex) {
    RenderPoseLog log;
    log.record(100, pose(0.1f));
    log.record(101, pose(0.2f));
    float q[4];
    ASSERT_TRUE(log.find(100, q));
    EXPECT_EQ(q[1], 0.1f);
    ASSERT_TRUE(log.find(101, q));
    EXPECT_EQ(q[1], 0.2f);
    EXPECT_FALSE(log.find(102, q)) << "never recorded";
}

TEST(RenderPoseLog, AnOldFrameIsForgottenNotConfused) {
    RenderPoseLog log;
    log.record(5, pose(0.5f));
    log.record(5 + RenderPoseLog::kSize, pose(0.9f));  // same slot
    float q[4];
    EXPECT_FALSE(log.find(5, q)) << "overwritten: not someone else's pose";
    ASSERT_TRUE(log.find(5 + RenderPoseLog::kSize, q));
    EXPECT_EQ(q[1], 0.9f);
}

TEST(RenderPoseLog, AnUnknownPoseIsNotFound) {
    RenderPoseLog log;
    log.record(7, fvp_client_protocol::FramePose{});
    float q[4];
    EXPECT_FALSE(log.find(7, q));
    log.record(8, pose(0.3f));
    log.clear();
    EXPECT_FALSE(log.find(8, q)) << "cleared for a new session";
}

TEST(RenderPoseLog, ThePresentationTimeGivesTheFrameIndex) {
    // Frames are submitted at frameIndex × frameDurationUs.
    EXPECT_EQ(RenderPoseLog::frameIndexOf(0, 11111), 0u);
    EXPECT_EQ(RenderPoseLog::frameIndexOf(1234 * 11111LL, 11111), 1234u);
    EXPECT_EQ(RenderPoseLog::frameIndexOf(1234 * 16666LL + 3, 16666), 1234u) << "rounding";
    EXPECT_EQ(RenderPoseLog::frameIndexOf(-1, 11111), UINT32_MAX) << "no frame yet";
    EXPECT_EQ(RenderPoseLog::frameIndexOf(100, 0), UINT32_MAX);
}
