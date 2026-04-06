#include "facial_tracker.h"
#include "xr_utils.h"
#include <cstring>

// XR_HTC_facial_tracking structs (inline to avoid SDK header dependency)
struct XrFacialTrackerCreateInfoHTC {
    XrStructureType type;
    const void* next;
    uint32_t facialTrackingType;
};

struct XrFacialExpressionsHTC {
    XrStructureType type;
    const void* next;
    XrBool32 isActive;
    XrTime sampleTime;
    uint32_t expressionCount;
    float* expressionWeightings;
};

// Structure type constants
static constexpr XrStructureType XR_TYPE_FACIAL_TRACKER_CREATE_INFO_HTC = (XrStructureType)1000104001;
static constexpr XrStructureType XR_TYPE_FACIAL_EXPRESSIONS_HTC = (XrStructureType)1000104002;

// Facial tracking types
static constexpr uint32_t XR_FACIAL_TRACKING_TYPE_EYE_DEFAULT_HTC = 1;
static constexpr uint32_t XR_FACIAL_TRACKING_TYPE_LIP_DEFAULT_HTC = 2;

// Function pointer types
typedef XrResult (*PFN_xrCreateFacialTrackerHTC)(XrSession, const XrFacialTrackerCreateInfoHTC*, uint64_t*);
typedef XrResult (*PFN_xrDestroyFacialTrackerHTC)(uint64_t);
typedef XrResult (*PFN_xrGetFacialExpressionsHTC)(uint64_t, XrFacialExpressionsHTC*);

bool FacialTracker::init(XrInstance instance, XrSession session) {
    m_session = session;

    // Resolve function pointers
    if (xrGetInstanceProcAddr(instance, "xrCreateFacialTrackerHTC", &m_pfnCreateFacialTracker) != XR_SUCCESS ||
        m_pfnCreateFacialTracker == nullptr) {
        LOGI("FacialTracker: XR_HTC_facial_tracking not available");
        return false;
    }
    xrGetInstanceProcAddr(instance, "xrDestroyFacialTrackerHTC", &m_pfnDestroyFacialTracker);
    xrGetInstanceProcAddr(instance, "xrGetFacialExpressionsHTC", &m_pfnGetFacialExpressions);

    auto createFn = (PFN_xrCreateFacialTrackerHTC)m_pfnCreateFacialTracker;

    // Create lip tracker
    XrFacialTrackerCreateInfoHTC lipInfo = {};
    lipInfo.type = XR_TYPE_FACIAL_TRACKER_CREATE_INFO_HTC;
    lipInfo.facialTrackingType = XR_FACIAL_TRACKING_TYPE_LIP_DEFAULT_HTC;
    if (createFn(session, &lipInfo, &m_lipTracker) == XR_SUCCESS) {
        m_lipAvailable = true;
        LOGI("FacialTracker: Lip tracking enabled (37 blendshapes)");
    }

    // Create eye expression tracker
    XrFacialTrackerCreateInfoHTC eyeInfo = {};
    eyeInfo.type = XR_TYPE_FACIAL_TRACKER_CREATE_INFO_HTC;
    eyeInfo.facialTrackingType = XR_FACIAL_TRACKING_TYPE_EYE_DEFAULT_HTC;
    if (createFn(session, &eyeInfo, &m_eyeTracker) == XR_SUCCESS) {
        m_eyeAvailable = true;
        LOGI("FacialTracker: Eye expression tracking enabled (14 blendshapes)");
    }

    if (!m_lipAvailable && !m_eyeAvailable) {
        LOGW("FacialTracker: No facial tracking available on this device");
        return false;
    }

    return true;
}

void FacialTracker::shutdown() {
    if (m_pfnDestroyFacialTracker) {
        auto destroyFn = (PFN_xrDestroyFacialTrackerHTC)m_pfnDestroyFacialTracker;
        if (m_lipTracker) destroyFn(m_lipTracker);
        if (m_eyeTracker) destroyFn(m_eyeTracker);
    }
    m_lipTracker = 0;
    m_eyeTracker = 0;
    m_lipAvailable = false;
    m_eyeAvailable = false;
}

FacialTracker::FaceData FacialTracker::poll() {
    FaceData data = {};

    if (!m_pfnGetFacialExpressions) return data;
    auto getFn = (PFN_xrGetFacialExpressionsHTC)m_pfnGetFacialExpressions;

    // Poll lip expressions
    if (m_lipAvailable && m_lipTracker) {
        XrFacialExpressionsHTC lipExpr = {};
        lipExpr.type = XR_TYPE_FACIAL_EXPRESSIONS_HTC;
        lipExpr.expressionCount = LIP_COUNT;
        lipExpr.expressionWeightings = data.lip.data();
        if (getFn(m_lipTracker, &lipExpr) == XR_SUCCESS && lipExpr.isActive) {
            data.lipValid = true;
        }
    }

    // Poll eye expressions
    if (m_eyeAvailable && m_eyeTracker) {
        XrFacialExpressionsHTC eyeExpr = {};
        eyeExpr.type = XR_TYPE_FACIAL_EXPRESSIONS_HTC;
        eyeExpr.expressionCount = EYE_COUNT;
        eyeExpr.expressionWeightings = data.eye.data();
        if (getFn(m_eyeTracker, &eyeExpr) == XR_SUCCESS && eyeExpr.isActive) {
            data.eyeValid = true;
        }
    }

    return data;
}
