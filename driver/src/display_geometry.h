#pragma once

#include <cmath>
#include <cstdint>

/**
 * What SteamVR needs to render for the headset: the per-eye field of view
 * and where each eye sits in the (virtual) display. Pure math, no OpenVR
 * calls, so it is unit-tested (tests/test_display_geometry.cpp).
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

/// Used until the headset reports its own field of view (TODOS: stereo and
/// FOV). A symmetric 100° per eye; the image is scaled wrong on the headset
/// by however much its real FOV differs.
inline constexpr Fov kDefaultFov{-50.0f, 50.0f, 50.0f, -50.0f};

/// `IVRDisplayComponent::GetProjectionRaw` values: tangents of the
/// half-angles, with y pointing down (so `top` is negative).
struct ProjectionRaw {
    float left;
    float right;
    float top;
    float bottom;
};

inline ProjectionRaw projectionRaw(const Fov& fov) {
    constexpr float kDegToRad = 3.14159265358979f / 180.0f;
    return {
        std::tan(fov.left * kDegToRad),
        std::tan(fov.right * kDegToRad),
        -std::tan(fov.up * kDegToRad),
        -std::tan(fov.down * kDegToRad),
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

}  // namespace fvp_display
