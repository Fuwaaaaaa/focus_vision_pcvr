#include "hmd_profile.h"
#include "xr_utils.h"
#include <algorithm>
#include <cstring>

static std::string toLower(const std::string& s) {
    std::string r = s;
    std::transform(r.begin(), r.end(), r.begin(), ::tolower);
    return r;
}

HmdProfile HmdProfile::detect(const char* systemName, const char* vendorName) {
    std::string sys = toLower(systemName ? systemName : "");
    std::string vendor = toLower(vendorName ? vendorName : "");

    // VIVE Focus Vision
    if (sys.find("focus") != std::string::npos && sys.find("vision") != std::string::npos) {
        return forType(HmdType::FocusVision);
    }
    if (vendor.find("htc") != std::string::npos && sys.find("focus") != std::string::npos) {
        return forType(HmdType::FocusVision);
    }

    // Meta Quest 3
    if (sys.find("quest 3s") != std::string::npos || sys.find("quest3s") != std::string::npos) {
        return forType(HmdType::Quest3S);
    }
    if (sys.find("quest 3") != std::string::npos || sys.find("quest3") != std::string::npos) {
        return forType(HmdType::Quest3);
    }
    if (sys.find("quest pro") != std::string::npos) {
        return forType(HmdType::QuestPro);
    }

    // Pico
    if (vendor.find("pico") != std::string::npos && sys.find("neo") != std::string::npos) {
        return forType(HmdType::PicoNeo4);
    }

    LOGW("HmdProfile: unrecognized HMD '%s' (vendor '%s'), using Generic", systemName, vendorName);
    return forType(HmdType::Generic);
}

HmdProfile HmdProfile::forType(HmdType type) {
    HmdProfile p;
    p.type = type;

    switch (type) {
    case HmdType::FocusVision:
        p.name = "VIVE Focus Vision";
        p.display = { 1832, 1920, 90, 0.063f, true, false };
        p.codec = { true, true, false, 150000, 80000 };
        break;

    case HmdType::Quest3:
        p.name = "Meta Quest 3";
        p.display = { 2064, 2208, 120, 0.063f, false, false };
        p.codec = { true, true, true, 200000, 100000 };
        break;

    case HmdType::Quest3S:
        p.name = "Meta Quest 3S";
        p.display = { 1832, 1920, 120, 0.063f, false, false };
        p.codec = { true, true, true, 150000, 80000 };
        break;

    case HmdType::QuestPro:
        p.name = "Meta Quest Pro";
        p.display = { 1800, 1920, 90, 0.063f, true, false };
        p.codec = { true, true, false, 150000, 80000 };
        break;

    case HmdType::VisionPro:
        p.name = "Apple Vision Pro";
        // Vision Pro has very high res — PCVR streaming typically uses lower
        p.display = { 2048, 2048, 90, 0.064f, true, true };
        p.codec = { true, true, false, 200000, 120000 };
        break;

    case HmdType::PicoNeo4:
        p.name = "Pico Neo 4";
        p.display = { 2160, 2160, 90, 0.063f, false, false };
        p.codec = { true, true, false, 150000, 80000 };
        break;

    case HmdType::Generic:
    default:
        p.name = "Generic HMD";
        // Conservative defaults that work on most devices
        p.display = { 1832, 1920, 90, 0.063f, false, false };
        p.codec = { true, true, false, 100000, 60000 };
        break;
    }

    LOGI("HmdProfile: %s (%ux%u @ %uHz, eyetrack=%s)",
         p.name.c_str(), p.display.widthPerEye, p.display.heightPerEye,
         p.display.refreshRate, p.display.hasEyeTracking ? "yes" : "no");

    return p;
}
