#pragma once

#include <openxr/openxr.h>
#include <cstdint>
#include <array>

/// Face tracking via OpenXR XR_HTC_facial_tracking.
/// Polls lip (37) and eye (14) blend shapes each frame.
/// Total: 51 float values sent to PC for VRCFaceTracking bridge.
class FacialTracker {
public:
    static constexpr uint32_t LIP_COUNT = 37;
    static constexpr uint32_t EYE_COUNT = 14;
    static constexpr uint32_t TOTAL_BLENDSHAPES = LIP_COUNT + EYE_COUNT; // 51

    struct FaceData {
        std::array<float, LIP_COUNT> lip;
        std::array<float, EYE_COUNT> eye;
        bool lipValid;
        bool eyeValid;
    };

    bool init(XrInstance instance, XrSession session);
    void shutdown();

    /// Poll current facial expressions. Call once per frame.
    FaceData poll();

    bool isAvailable() const { return m_lipAvailable || m_eyeAvailable; }
    bool isLipAvailable() const { return m_lipAvailable; }
    bool isEyeAvailable() const { return m_eyeAvailable; }

private:
    bool m_lipAvailable = false;
    bool m_eyeAvailable = false;
    XrSession m_session = XR_NULL_HANDLE;

    // Opaque handles — stored as void* for SDK version compatibility.
    // Cast to XrFacialTrackerHTC at call site.
    void* m_lipTracker = nullptr;
    void* m_eyeTracker = nullptr;

    // Function pointers (resolved via xrGetInstanceProcAddr)
    PFN_xrVoidFunction m_pfnCreateFacialTracker = nullptr;
    PFN_xrVoidFunction m_pfnDestroyFacialTracker = nullptr;
    PFN_xrVoidFunction m_pfnGetFacialExpressions = nullptr;
};
