#pragma once

#include <cmath>
#include <cstdint>

/**
 * What SteamVR needs to render for the headset: the per-eye field of view,
 * where each eye sits in the (virtual) display, and where the eyes are
 * relative to the head. Pure math, no OpenVR calls, so it is unit-tested
 * (tests/test_display_geometry.cpp).
 */
namespace fvp_display {

/// Per-eye field of view in degrees, OpenXR convention: `left` and `down`
/// are negative.
struct Fov {
    float left;
    float right;
    float up;
    float down;
};

/// Used until the headset reports its own field of view (VIEW_CONFIG): a
/// symmetric 100° per eye.
inline constexpr Fov kDefaultFov{-50.0f, 50.0f, 50.0f, -50.0f};

/// An eye's field of view from the radians the headset sends (XrFovf).
inline Fov fovFromRadians(float left, float right, float up, float down) {
    constexpr float kRadToDeg = 180.0f / 3.14159265358979f;
    return {left * kRadToDeg, right * kRadToDeg, up * kRadToDeg, down * kRadToDeg};
}

/// `IVRDisplayComponent::GetProjectionRaw` / `SetDisplayProjectionRaw`
/// values: tangents of the half-angles.
struct ProjectionRaw {
    float left;
    float right;
    float top;
    float bottom;
};

/// OpenVR builds its projection with ComposeProjection, which puts `bottom`
/// at the top of the image (NDC y = +1) and `top` at its bottom: `top` is
/// the tangent of the lower edge (negative) and `bottom` of the upper edge.
/// ALVR's fov_to_tangents does the same. (Swapping them flips a headset's
/// vertical asymmetry — they differ only for an asymmetric field of view.)
inline ProjectionRaw projectionRaw(const Fov& fov) {
    constexpr float kDegToRad = 3.14159265358979f / 180.0f;
    return {
        std::tan(fov.left * kDegToRad),
        std::tan(fov.right * kDegToRad),
        std::tan(fov.down * kDegToRad),
        std::tan(fov.up * kDegToRad),
    };
}

struct Viewport {
    uint32_t x;
    uint32_t y;
    uint32_t width;
    uint32_t height;
};

/// The eyes side by side in the virtual display, left eye on the left.
/// `eye`: 0 = left, 1 = right.
inline Viewport eyeViewport(int eye, uint32_t eyeWidth, uint32_t eyeHeight) {
    return {eye == 0 ? 0u : eyeWidth, 0u, eyeWidth, eyeHeight};
}

/// Row-major 3x4 eye-to-head transform (`HmdMatrix34_t` layout): the eye
/// `x` metres right of the head's centre (negative = left), looking
/// straight ahead — the Focus Vision's displays are parallel.
struct EyeToHead {
    float m[3][4];
};

inline EyeToHead eyeToHead(float x) {
    return {{{1.0f, 0.0f, 0.0f, x}, {0.0f, 1.0f, 0.0f, 0.0f}, {0.0f, 0.0f, 1.0f, 0.0f}}};
}

}  // namespace fvp_display
