#pragma once

#include <cmath>
#include "display_geometry.h"

/**
 * Lining a frame's layers up with its first. SteamVR submits the scene and,
 * above it, overlays and the dashboard, each rendered for the head pose in
 * its `mHmdPose`: when an app falls behind, the compositor draws its
 * overlays at a newer pose than the app's scene. The stream carries one
 * image per eye at the scene's pose, so each layer above it is turned to
 * that pose before it is blended on: a pixel of the scene's eye image is a
 * ray, looked up in the layer's eye image after the rotation between the
 * two head orientations. Orientation only — the head's movement between
 * the poses is small next to the distance of what the layers show (ALVR
 * does the same). Pure math, mirrored by EyeBlit's shader and unit-tested
 * (tests/test_layer_compose.cpp).
 */
namespace fvp_layers {

/// An eye image's edges as tangents: x to the right, y up (so `left` and
/// `down` are negative).
struct Tangents {
    float left;
    float right;
    float up;
    float down;
};

inline Tangents tangents(const fvp_display::Fov& fov) {
    constexpr float kDegToRad = 3.14159265358979f / 180.0f;
    return {std::tan(fov.left * kDegToRad), std::tan(fov.right * kDegToRad), std::tan(fov.up * kDegToRad),
            std::tan(fov.down * kDegToRad)};
}

/// Row-major 3x3.
struct Mat3 {
    float m[3][3];
};

inline constexpr Mat3 kIdentity{{{1.0f, 0.0f, 0.0f}, {0.0f, 1.0f, 0.0f}, {0.0f, 0.0f, 1.0f}}};

/// The rotation of a pose (the upper-left 3x3 of a row-major 3x4
/// `HmdMatrix34_t`), if it is one: false for a missing (zero) pose or one
/// that isn't a rotation.
inline bool rotationOf(const float pose[3][4], Mat3& out) {
    for (int r = 0; r < 3; r++)
        for (int c = 0; c < 3; c++) out.m[r][c] = pose[r][c];
    constexpr float kTolerance = 1e-2f;
    for (int a = 0; a < 3; a++) {
        for (int b = a; b < 3; b++) {
            const float dot = out.m[a][0] * out.m[b][0] + out.m[a][1] * out.m[b][1] + out.m[a][2] * out.m[b][2];
            if (std::fabs(dot - (a == b ? 1.0f : 0.0f)) > kTolerance) return false;
        }
    }
    const float det = out.m[0][0] * (out.m[1][1] * out.m[2][2] - out.m[1][2] * out.m[2][1]) -
                      out.m[0][1] * (out.m[1][0] * out.m[2][2] - out.m[1][2] * out.m[2][0]) +
                      out.m[0][2] * (out.m[1][0] * out.m[2][1] - out.m[1][1] * out.m[2][0]);
    return det > 0.0f;  // not a reflection
}

/// How a layer above the scene lines up with it.
struct Placement {
    bool rotated = false;       // rendered at another head orientation
    Mat3 rotation = kIdentity;  // a direction in the scene's head frame → the layer's
};

/// R_layer^T · R_scene. Not rotated when the orientations match or either
/// pose is unusable: the layer is then drawn as it is.
inline Placement place(const float scenePose[3][4], const float layerPose[3][4]) {
    Mat3 scene, layer;
    Placement p;
    if (!rotationOf(scenePose, scene) || !rotationOf(layerPose, layer)) return p;
    bool same = true;
    for (int r = 0; r < 3; r++) {
        for (int c = 0; c < 3; c++) {
            p.rotation.m[r][c] = layer.m[0][r] * scene.m[0][c] + layer.m[1][r] * scene.m[1][c] +
                                 layer.m[2][r] * scene.m[2][c];
            // 1e-5 of a unit vector is 0.0006°, well below a pixel.
            if (std::fabs(p.rotation.m[r][c] - kIdentity.m[r][c]) > 1e-5f) same = false;
        }
    }
    p.rotated = !same;
    if (same) p.rotation = kIdentity;
    return p;
}

/// The point (u, v) of the layer's eye image (0..1, v down) on the ray
/// through point (x, y) of the scene's eye image, both with the field of
/// view `eye`. False when that ray misses the layer's image.
inline bool layerPoint(const Placement& p, const Tangents& eye, float x, float y, float& u, float& v) {
    const float d[3] = {eye.left + (eye.right - eye.left) * x, eye.up + (eye.down - eye.up) * y, -1.0f};
    float r[3];
    for (int i = 0; i < 3; i++) r[i] = p.rotation.m[i][0] * d[0] + p.rotation.m[i][1] * d[1] + p.rotation.m[i][2] * d[2];
    if (r[2] > -1e-4f) return false;  // at or behind the layer's eye plane
    const float px = r[0] / -r[2];
    const float py = r[1] / -r[2];
    u = (px - eye.left) / (eye.right - eye.left);
    v = (eye.up - py) / (eye.up - eye.down);
    return u >= 0.0f && u <= 1.0f && v >= 0.0f && v <= 1.0f;
}

}  // namespace fvp_layers
