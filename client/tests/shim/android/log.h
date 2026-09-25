#pragma once
// Host-test shim for the NDK's <android/log.h>. Lets client sources that only
// log (e.g. fec_decoder.cpp via xr_utils.h) build with a desktop toolchain.
// Only on the include path of client/tests — never of the NDK build.
#include <cstdio>

enum {
    ANDROID_LOG_INFO = 4,
    ANDROID_LOG_WARN = 5,
    ANDROID_LOG_ERROR = 6,
};

inline int __android_log_print(int /*prio*/, const char* /*tag*/, const char* /*fmt*/, ...) {
    return 0;
}
