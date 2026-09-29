#pragma once

#include <openvr_driver.h>
#include "display_geometry.h"

/**
 * The headset's display as SteamVR's compositor sees it: how large to
 * render each eye, with which projection, and where the eyes sit. There is
 * no real display — the frames go to the encoder through direct mode — and
 * no lens distortion (the headset's runtime applies its own).
 */
class CDisplayComponent : public vr::IVRDisplayComponent {
public:
    /// `fov` is the left eye's; the right eye mirrors it.
    void configure(uint32_t eyeWidth, uint32_t eyeHeight, const fvp_display::Fov& fov) {
        m_eyeWidth = eyeWidth;
        m_eyeHeight = eyeHeight;
        m_fov = fov;
    }

    void GetWindowBounds(int32_t* pnX, int32_t* pnY, uint32_t* pnWidth, uint32_t* pnHeight) override {
        *pnX = 0;
        *pnY = 0;
        *pnWidth = m_eyeWidth * 2;
        *pnHeight = m_eyeHeight;
    }

    bool IsDisplayOnDesktop() override { return false; }
    bool IsDisplayRealDisplay() override { return false; }

    void GetRecommendedRenderTargetSize(uint32_t* pnWidth, uint32_t* pnHeight) override {
        *pnWidth = m_eyeWidth;
        *pnHeight = m_eyeHeight;
    }

    void GetEyeOutputViewport(vr::EVREye eEye, uint32_t* pnX, uint32_t* pnY, uint32_t* pnWidth,
                              uint32_t* pnHeight) override {
        const fvp_display::Viewport v =
            fvp_display::eyeViewport(eEye == vr::Eye_Left ? 0 : 1, m_eyeWidth, m_eyeHeight);
        *pnX = v.x;
        *pnY = v.y;
        *pnWidth = v.width;
        *pnHeight = v.height;
    }

    void GetProjectionRaw(vr::EVREye eEye, float* pfLeft, float* pfRight, float* pfTop,
                          float* pfBottom) override {
        fvp_display::Fov fov = m_fov;
        if (eEye == vr::Eye_Right) fov = {-m_fov.right, -m_fov.left, m_fov.up, m_fov.down};
        const fvp_display::ProjectionRaw p = fvp_display::projectionRaw(fov);
        *pfLeft = p.left;
        *pfRight = p.right;
        *pfTop = p.top;
        *pfBottom = p.bottom;
    }

    vr::DistortionCoordinates_t ComputeDistortion(vr::EVREye, float fU, float fV) override {
        vr::DistortionCoordinates_t d{};
        d.rfRed[0] = d.rfGreen[0] = d.rfBlue[0] = fU;
        d.rfRed[1] = d.rfGreen[1] = d.rfBlue[1] = fV;
        return d;
    }

    bool ComputeInverseDistortion(vr::HmdVector2_t*, vr::EVREye, uint32_t, float, float) override {
        return false;  // none to invert
    }

private:
    uint32_t m_eyeWidth = 1832;
    uint32_t m_eyeHeight = 1920;
    fvp_display::Fov m_fov = fvp_display::kDefaultFov;
};
