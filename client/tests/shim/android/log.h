#pragma once
// Host-test shim for the NDK's <android/log.h>. Lets client sources that only
// log (fec_decoder.cpp, tcp_client.cpp, ... via xr_utils.h) build with a
// desktop toolchain. Only on the include path of client/tests — never of the
// NDK build. Silent unless FVP_TEST_LOG is set, then it prints to stderr.
#include <cstdarg>
#include <cstdio>
#include <cstdlib>

enum {
    ANDROID_LOG_INFO = 4,
    ANDROID_LOG_WARN = 5,
    ANDROID_LOG_ERROR = 6,
};

inline int __android_log_print(int prio, const char* tag, const char* fmt, ...) {
    static const bool enabled = std::getenv("FVP_TEST_LOG") != nullptr;
    if (!enabled) return 0;
    const char level = prio >= ANDROID_LOG_ERROR ? 'E' : prio == ANDROID_LOG_WARN ? 'W' : 'I';
    std::fprintf(stderr, "%c/%s: ", level, tag);
    va_list args;
    va_start(args, fmt);
    std::vfprintf(stderr, fmt, args);
    va_end(args);
    std::fputc('\n', stderr);
    return 0;
}
