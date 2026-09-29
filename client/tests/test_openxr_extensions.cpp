// Which OpenXR extensions the client asks for (openxr_extensions.h).
// REGRESSION: the instance enabled only the two required extensions, so the
// Focus 3 controller profile, eye gaze and facial tracking were unusable
// whatever the runtime offered.
#include <gtest/gtest.h>

#include <string>
#include <vector>

#include "openxr_extensions.h"

using namespace fvp_xr;

namespace {
bool has(const Extensions& e, const char* name) {
    for (const char* n : e.names) {
        if (std::string(n) == name) return true;
    }
    return false;
}
}  // namespace

TEST(OpenXrExtensions, EnablesEverythingTheFocusVisionOffers) {
    const std::vector<std::string> runtime = {
        "XR_KHR_opengl_es_enable", "XR_KHR_android_create_instance",
        "XR_HTC_vive_focus3_controller_interaction", "XR_EXT_eye_gaze_interaction",
        "XR_HTC_facial_tracking", "XR_KHR_composition_layer_depth",
    };
    const Extensions e = chooseExtensions(runtime);
    EXPECT_FALSE(e.missingRequired);
    EXPECT_TRUE(e.focus3Controller);
    EXPECT_TRUE(e.eyeGaze);
    EXPECT_TRUE(e.facialTracking);
    EXPECT_EQ(e.names.size(), 5u) << "nothing the app doesn't use";
    EXPECT_TRUE(has(e, "XR_HTC_vive_focus3_controller_interaction"));
    EXPECT_FALSE(has(e, "XR_KHR_composition_layer_depth"));
}

TEST(OpenXrExtensions, SkipsWhatTheRuntimeLacks) {
    // Asking for an extension the runtime doesn't have fails xrCreateInstance.
    const Extensions e = chooseExtensions({"XR_KHR_opengl_es_enable", "XR_KHR_android_create_instance"});
    EXPECT_FALSE(e.missingRequired);
    EXPECT_FALSE(e.focus3Controller);
    EXPECT_FALSE(e.eyeGaze);
    EXPECT_FALSE(e.facialTracking);
    EXPECT_EQ(e.names.size(), 2u);
}

TEST(OpenXrExtensions, ReportsAMissingRequiredExtension) {
    const Extensions e = chooseExtensions({"XR_KHR_android_create_instance", "XR_EXT_eye_gaze_interaction"});
    EXPECT_TRUE(e.missingRequired);
    EXPECT_TRUE(has(e, "XR_KHR_opengl_es_enable")) << "still requested, so the failure names it";
    EXPECT_TRUE(e.eyeGaze);
}
