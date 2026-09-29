#include <gtest/gtest.h>

#include <cmath>

#include "layer_compose.h"

using fvp_layers::layerPoint;
using fvp_layers::place;
using fvp_layers::Placement;
using fvp_layers::Tangents;

namespace {

constexpr float kDegToRad = 3.14159265358979f / 180.0f;

/// A row-major 3x4 head pose (HmdMatrix34_t layout), rotation only.
struct Pose {
    float m[3][4];
};

Pose rotation(float r00, float r01, float r02, float r10, float r11, float r12, float r20, float r21, float r22) {
    return {{{r00, r01, r02, 0.0f}, {r10, r11, r12, 0.0f}, {r20, r21, r22, 0.0f}}};
}

/// The head turned left by `degrees` (about +Y; OpenVR looks down -Z).
Pose yaw(float degrees) {
    const float c = std::cos(degrees * kDegToRad), s = std::sin(degrees * kDegToRad);
    return rotation(c, 0, s, 0, 1, 0, -s, 0, c);
}

/// The head tilted up by `degrees` (about +X).
Pose pitch(float degrees) {
    const float c = std::cos(degrees * kDegToRad), s = std::sin(degrees * kDegToRad);
    return rotation(1, 0, 0, 0, c, -s, 0, s, c);
}

const Tangents kSymmetric = fvp_layers::tangents({-50.0f, 50.0f, 50.0f, -50.0f});

}  // namespace

TEST(LayerCompose, TangentsOfTheFieldOfView) {
    const Tangents t = fvp_layers::tangents({-40.0f, 50.0f, 45.0f, -52.0f});
    EXPECT_NEAR(t.left, -std::tan(40.0f * kDegToRad), 1e-6f);
    EXPECT_NEAR(t.right, std::tan(50.0f * kDegToRad), 1e-6f);
    EXPECT_NEAR(t.up, 1.0f, 1e-6f);
    EXPECT_NEAR(t.down, -std::tan(52.0f * kDegToRad), 1e-6f);
}

TEST(LayerCompose, ALayerAtTheScenesPoseIsDrawnAsItIs) {
    const Pose head = yaw(37.0f);
    const Placement p = place(head.m, head.m);
    EXPECT_FALSE(p.rotated);

    // Even with an asymmetric field of view, each point maps to itself.
    const Tangents eye = fvp_layers::tangents({-40.0f, 50.0f, 45.0f, -52.0f});
    float u = 0, v = 0;
    ASSERT_TRUE(layerPoint(p, eye, 0.2f, 0.7f, u, v));
    EXPECT_NEAR(u, 0.2f, 1e-5f);
    EXPECT_NEAR(v, 0.7f, 1e-5f);
}

TEST(LayerCompose, ALayerRenderedWithTheHeadTurnedLeftIsLookedUpToTheRight) {
    // The layer was drawn with the head 10° further left: straight ahead of
    // the scene is 10° right of the layer's centre.
    const Pose scene = yaw(0.0f);
    const Pose layer = yaw(10.0f);
    const Placement p = place(scene.m, layer.m);
    ASSERT_TRUE(p.rotated);
    float u = 0, v = 0;
    ASSERT_TRUE(layerPoint(p, kSymmetric, 0.5f, 0.5f, u, v));
    EXPECT_NEAR(u, 0.5f + std::tan(10.0f * kDegToRad) / (2.0f * kSymmetric.right), 1e-5f);
    EXPECT_NEAR(v, 0.5f, 1e-5f);
}

TEST(LayerCompose, ALayerRenderedWithTheHeadTiltedUpIsLookedUpLower) {
    const Placement p = place(pitch(0.0f).m, pitch(10.0f).m);
    ASSERT_TRUE(p.rotated);
    float u = 0, v = 0;
    ASSERT_TRUE(layerPoint(p, kSymmetric, 0.5f, 0.5f, u, v));
    EXPECT_NEAR(u, 0.5f, 1e-5f);
    EXPECT_NEAR(v, 0.5f + std::tan(10.0f * kDegToRad) / (2.0f * kSymmetric.up), 1e-5f) << "v grows downwards";
}

TEST(LayerCompose, OnlyTheDifferenceBetweenThePosesCounts) {
    const Placement relative = place(yaw(30.0f).m, yaw(40.0f).m);
    const Placement fromAhead = place(yaw(0.0f).m, yaw(10.0f).m);
    float u1 = 0, v1 = 0, u2 = 0, v2 = 0;
    ASSERT_TRUE(layerPoint(relative, kSymmetric, 0.3f, 0.6f, u1, v1));
    ASSERT_TRUE(layerPoint(fromAhead, kSymmetric, 0.3f, 0.6f, u2, v2));
    EXPECT_NEAR(u1, u2, 1e-5f);
    EXPECT_NEAR(v1, v2, 1e-5f);
}

TEST(LayerCompose, RaysOutsideTheLayersImageMissIt) {
    float u = 0, v = 0;
    // 30° apart with ±50° eyes: the scene's right edge (50°) is 80° into
    // the layer, past its edge; its left edge is inside.
    const Placement p = place(yaw(0.0f).m, yaw(30.0f).m);
    EXPECT_FALSE(layerPoint(p, kSymmetric, 1.0f, 0.5f, u, v));
    EXPECT_TRUE(layerPoint(p, kSymmetric, 0.0f, 0.5f, u, v));
    // Turned right round: behind the layer's eye.
    EXPECT_FALSE(layerPoint(place(yaw(0.0f).m, yaw(180.0f).m), kSymmetric, 0.5f, 0.5f, u, v));
}

TEST(LayerCompose, AnUnusablePoseLeavesTheLayerAsItIs) {
    const Pose zero{};
    EXPECT_FALSE(place(yaw(0.0f).m, zero.m).rotated) << "missing pose";
    EXPECT_FALSE(place(zero.m, yaw(20.0f).m).rotated) << "missing scene pose";
    const Pose mirrored = rotation(-1, 0, 0, 0, 1, 0, 0, 0, 1);
    EXPECT_FALSE(place(yaw(0.0f).m, mirrored.m).rotated) << "a reflection is not a head pose";
    const Pose scaled = rotation(2, 0, 0, 0, 2, 0, 0, 0, 2);
    EXPECT_FALSE(place(yaw(0.0f).m, scaled.m).rotated);
}

TEST(LayerCompose, APoseWithTranslationStillTurns) {
    // Only the rotation is used; the head's position is ignored.
    Pose layer = yaw(10.0f);
    layer.m[0][3] = 0.3f;
    layer.m[1][3] = 1.7f;
    layer.m[2][3] = -0.2f;
    const Placement moved = place(yaw(0.0f).m, layer.m);
    const Placement turned = place(yaw(0.0f).m, yaw(10.0f).m);
    for (int r = 0; r < 3; r++)
        for (int c = 0; c < 3; c++) EXPECT_FLOAT_EQ(moved.rotation.m[r][c], turned.rotation.m[r][c]);
}

TEST(LayerCompose, APoseBecomesTheQuaternionOfItsRotation) {
    // The orientation sent with each frame (v6).
    float q[4];
    ASSERT_TRUE(fvp_layers::orientationOf(yaw(0.0f).m, q));
    EXPECT_NEAR(q[3], 1.0f, 1e-6f);
    EXPECT_NEAR(q[0], 0.0f, 1e-6f);

    ASSERT_TRUE(fvp_layers::orientationOf(yaw(40.0f).m, q));
    EXPECT_NEAR(q[0], 0.0f, 1e-6f);
    EXPECT_NEAR(q[1], std::sin(20.0f * kDegToRad), 1e-6f) << "turned left: about +Y";
    EXPECT_NEAR(q[2], 0.0f, 1e-6f);
    EXPECT_NEAR(q[3], std::cos(20.0f * kDegToRad), 1e-6f);

    ASSERT_TRUE(fvp_layers::orientationOf(pitch(-30.0f).m, q));
    EXPECT_NEAR(q[0], std::sin(-15.0f * kDegToRad), 1e-6f) << "tilted down: about -X";
    EXPECT_NEAR(q[3], std::cos(-15.0f * kDegToRad), 1e-6f);

    // Turned right round (trace -1): the other branches.
    ASSERT_TRUE(fvp_layers::orientationOf(yaw(180.0f).m, q));
    EXPECT_NEAR(std::fabs(q[1]), 1.0f, 1e-5f);
    EXPECT_NEAR(q[3], 0.0f, 1e-5f);

    const Pose zero{};
    EXPECT_FALSE(fvp_layers::orientationOf(zero.m, q)) << "no pose, no orientation";
}

TEST(LayerCompose, TheQuaternionRotatesLikeThePose) {
    // Any rotation: rotating a vector with q equals the pose's matrix.
    const Pose p = rotation(0.36f, 0.48f, -0.8f, -0.8f, 0.6f, 0.0f, 0.48f, 0.64f, 0.6f);
    float q[4];
    ASSERT_TRUE(fvp_layers::orientationOf(p.m, q));
    const float v[3] = {0.3f, -0.5f, 0.8f};
    // v' = v + 2w (u × v) + 2 u × (u × v), u = (x, y, z)
    const float u[3] = {q[0], q[1], q[2]};
    const float uv[3] = {u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]};
    const float uuv[3] = {u[1] * uv[2] - u[2] * uv[1], u[2] * uv[0] - u[0] * uv[2], u[0] * uv[1] - u[1] * uv[0]};
    for (int i = 0; i < 3; i++) {
        const float byQ = v[i] + 2.0f * q[3] * uv[i] + 2.0f * uuv[i];
        const float byM = p.m[i][0] * v[0] + p.m[i][1] * v[1] + p.m[i][2] * v[2];
        EXPECT_NEAR(byQ, byM, 1e-5f) << "component " << i;
    }
}
