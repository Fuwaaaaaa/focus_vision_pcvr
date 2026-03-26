#pragma once

#include <openxr/openxr.h>
#include <android/log.h>
#include <string>
#include <stdexcept>

#define LOG_TAG "FocusVision"
#define LOGI(...) __android_log_print(ANDROID_LOG_INFO, LOG_TAG, __VA_ARGS__)
#define LOGW(...) __android_log_print(ANDROID_LOG_WARN, LOG_TAG, __VA_ARGS__)
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, LOG_TAG, __VA_ARGS__)

/// Check XrResult and throw on failure
inline void xrCheck(XrResult result, const char* msg) {
    if (XR_FAILED(result)) {
        char buf[256];
        snprintf(buf, sizeof(buf), "OpenXR error: %s (result=%d)", msg, (int)result);
        LOGE("%s", buf);
        throw std::runtime_error(buf);
    }
}

#define XR_CHECK(result, msg) xrCheck(result, msg)
