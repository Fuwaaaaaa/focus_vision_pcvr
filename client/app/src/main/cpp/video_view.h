#pragma once
// Where each eye's image sits in a decoded video frame, and the per-pixel
// rotational reprojection the renderer applies when it shows an older frame.
// Free of GL / OpenXR so the host tests check the math the fragment shader
// in renderer.cpp runs (client/tests/test_video_view.cpp).
#include <cmath>
#include <cstdint>

#include "client_protocol.h"

namespace fvp_video {

/// A region of the decoded frame in texture UV, v growing downwards from
/// the top of the image.
struct UvRect {
    float u0 = 0.0f;
    float v0 = 0.0f;
    float u1 = 1.0f;
    float v1 = 1.0f;
};

/// The part of the frame eye `eye` (0 = left, 1 = right) shows: its half of
/// a side-by-side frame, or the whole of a mono one.
inline UvRect eyeRect(uint8_t layout, int eye) {
    if (layout != fvp_client_protocol::STEREO_SIDE_BY_SIDE) return {};
    return eye == 0 ? UvRect{0.0f, 0.0f, 0.5f, 1.0f} : UvRect{0.5f, 0.0f, 1.0f, 1.0f};
}

/// Tangents of an eye's field of view (from XrFovf's angles, radians):
/// `left` and `down` are negative.
struct Tangents {
    float left;
    float right;
    float down;
    float up;
};

inline Tangents tangents(float angleLeft, float angleRight, float angleDown, float angleUp) {
    return {std::tan(angleLeft), std::tan(angleRight), std::tan(angleDown), std::tan(angleUp)};
}

struct Quat {
    float x;
    float y;
    float z;
    float w;
};

/// Row-major 3x3 rotation of `q` (x, y, z, w).
inline void rotationMatrix(const Quat& q, float m[9]) {
    const float xx = q.x * q.x, yy = q.y * q.y, zz = q.z * q.z;
    const float xy = q.x * q.y, xz = q.x * q.z, yz = q.y * q.z;
    const float wx = q.w * q.x, wy = q.w * q.y, wz = q.w * q.z;
    m[0] = 1 - 2 * (yy + zz); m[1] = 2 * (xy - wz);     m[2] = 2 * (xz + wy);
    m[3] = 2 * (xy + wz);     m[4] = 1 - 2 * (xx + zz); m[5] = 2 * (yz - wx);
    m[6] = 2 * (xz - wy);     m[7] = 2 * (yz + wx);     m[8] = 1 - 2 * (xx + yy);
}

/// Row-major rotation taking a direction in the current view to the view
/// the frame was drawn for: inverse(frame) * current. OpenXR view
/// orientations rotate view space into the reference space.
inline void reprojectionRotation(const Quat& frame, const Quat& current, float m[9]) {
    const Quat inv{-frame.x, -frame.y, -frame.z, frame.w};  // unit quaternion
    const Quat d{
        inv.w * current.x + inv.x * current.w + inv.y * current.z - inv.z * current.y,
        inv.w * current.y - inv.x * current.z + inv.y * current.w + inv.z * current.x,
        inv.w * current.z + inv.x * current.y - inv.y * current.x + inv.z * current.w,
        inv.w * current.w - inv.x * current.x - inv.y * current.y - inv.z * current.z,
    };
    rotationMatrix(d, m);
}

inline constexpr float kIdentity[9] = {1, 0, 0, 0, 1, 0, 0, 0, 1};

/// The UV within the eye's image (0..1, v down) that the pixel at `ndcX`,
/// `ndcY` (-1..1, y up) of the current view shows, given the rotation from
/// the current view to the frame's (`rot`, row-major) and the eye's field of
/// view. False when that direction falls outside the frame: the pixel is
/// drawn black. The fragment shader in renderer.cpp does the same.
inline bool sampleUv(float ndcX, float ndcY, const float rot[9], const Tangents& t, float& u, float& v) {
    // The pixel's direction in the current view: OpenXR views look down -Z.
    const float tx = t.left + (ndcX * 0.5f + 0.5f) * (t.right - t.left);
    const float ty = t.down + (ndcY * 0.5f + 0.5f) * (t.up - t.down);
    const float dx = rot[0] * tx + rot[1] * ty - rot[2];
    const float dy = rot[3] * tx + rot[4] * ty - rot[5];
    const float dz = rot[6] * tx + rot[7] * ty - rot[8];
    if (dz >= -1e-6f) return false;  // behind the frame's view
    const float rx = dx / -dz;
    const float ry = dy / -dz;
    u = (rx - t.left) / (t.right - t.left);
    v = 1.0f - (ry - t.down) / (t.up - t.down);
    return u >= 0.0f && u <= 1.0f && v >= 0.0f && v <= 1.0f;
}

/// Where a gaze orientation (view space: OpenXR looks down -Z, +Y up) falls
/// in an eye's image: `x` 0..1 left to right, `y` 0..1 top to bottom, as
/// the foveated QP map takes it. Clamped to the image; a gaze at or behind
/// 90° lands on the edge it points towards.
inline void gazeInImage(const Quat& gaze, const Tangents& t, float& x, float& y) {
    // The gaze direction: (0, 0, -1) rotated by `gaze`.
    float m[9];
    rotationMatrix(gaze, m);
    const float dx = -m[2];
    const float dy = -m[5];
    float dz = -m[8];
    if (dz > -1e-3f) dz = -1e-3f;
    const float tx = dx / -dz;
    const float ty = dy / -dz;
    auto clamp01 = [](float v) { return v < 0.0f ? 0.0f : (v > 1.0f ? 1.0f : v); };
    x = clamp01((tx - t.left) / (t.right - t.left));
    y = clamp01((t.up - ty) / (t.up - t.down));
}

/// `uv` within the eye's image → texture coordinate in the decoded frame.
inline void frameUv(const UvRect& eye, float u, float v, float& fu, float& fv) {
    fu = eye.u0 + u * (eye.u1 - eye.u0);
    fv = eye.v0 + v * (eye.v1 - eye.v0);
}

}  // namespace fvp_video
