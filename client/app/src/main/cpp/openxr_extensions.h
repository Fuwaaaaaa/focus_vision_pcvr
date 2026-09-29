#pragma once
// Which OpenXR instance extensions the client enables, from what the runtime
// offers. An extension must be enabled at xrCreateInstance for its
// interaction profile, paths and functions to exist — the Focus 3 controller
// bindings, eye gaze and facial tracking all failed while only the two
// required extensions were enabled. Pure, host-tested
// (client/tests/test_openxr_extensions.cpp).
#include <algorithm>
#include <string>
#include <vector>

namespace fvp_xr {

// Names as in openxr.h / openxr_platform.h (XR_*_EXTENSION_NAME).
inline constexpr const char* kOpenGlEs = "XR_KHR_opengl_es_enable";
inline constexpr const char* kAndroidCreateInstance = "XR_KHR_android_create_instance";
inline constexpr const char* kFocus3Controller = "XR_HTC_vive_focus3_controller_interaction";
inline constexpr const char* kEyeGaze = "XR_EXT_eye_gaze_interaction";
inline constexpr const char* kFacialTracking = "XR_HTC_facial_tracking";

struct Extensions {
    std::vector<const char*> names;  // for XrInstanceCreateInfo::enabledExtensionNames
    bool focus3Controller = false;
    bool eyeGaze = false;
    bool facialTracking = false;
    /// A required one is missing: the runtime can't run the app.
    bool missingRequired = false;
};

/// The required extensions, plus each optional one the runtime offers.
inline Extensions chooseExtensions(const std::vector<std::string>& available) {
    auto offered = [&](const char* name) {
        return std::find(available.begin(), available.end(), name) != available.end();
    };
    Extensions e;
    for (const char* required : {kOpenGlEs, kAndroidCreateInstance}) {
        e.names.push_back(required);
        if (!offered(required)) e.missingRequired = true;
    }
    auto optional = [&](const char* name, bool& flag) {
        flag = offered(name);
        if (flag) e.names.push_back(name);
    };
    optional(kFocus3Controller, e.focus3Controller);
    optional(kEyeGaze, e.eyeGaze);
    optional(kFacialTracking, e.facialTracking);
    return e;
}

}  // namespace fvp_xr
