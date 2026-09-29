#pragma once

#include <openvr_driver.h>
#include <cstdarg>
#include <cstdio>

/// One line in SteamVR's log (vrserver.txt), prefixed with the driver name.
inline void driverLog(const char* format, ...) {
    char message[480];
    va_list args;
    va_start(args, format);
    vsnprintf(message, sizeof(message), format, args);
    va_end(args);
    if (vr::IVRDriverLog* log = vr::VRDriverLog()) {
        char line[512];
        snprintf(line, sizeof(line), "Focus Vision PCVR: %s\n", message);
        log->Log(line);
    }
}

/// driverLog for callbacks that take a plain message (NvencEncoder::LogFn).
inline void driverLogMessage(const char* message) {
    driverLog("%s", message);
}
