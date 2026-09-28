#pragma once
// Where to connect, handed over at launch. The companion app starts the
// client over adb with the PC's address and the pairing PIN:
//
//   am start -n com.focusvision.pcvr/.MainActivity \
//       --es fvp_server 192.168.1.10 --es fvp_pin 012345 [--es fvp_udp_port 9945]
//
// MainActivity writes the extras to LAUNCH_REQUEST_FILE in app-private
// storage ("key=value" lines); the native loop reads, deletes and parses it.
// Pure: no Android dependency, unit-tested on the host.
#include "client_protocol.h"
#include "client_session.h"

#include <cstdint>
#include <string>

namespace fvp_launch {

inline constexpr const char* LAUNCH_REQUEST_FILE = "launch_request.txt";

struct LaunchRequest {
    fvp_session::ServerEndpoint server;
    uint32_t pin = 0;
    uint16_t udpBasePort = fvp_client_protocol::DEFAULT_UDP_BASE_PORT;
};

/// A PIN is six decimal digits (leading zeros kept, e.g. "012345").
inline bool parsePin(const std::string& s, uint32_t& out) {
    if (s.size() != 6) return false;
    uint32_t v = 0;
    for (char c : s) {
        if (c < '0' || c > '9') return false;
        v = v * 10 + static_cast<uint32_t>(c - '0');
    }
    out = v;
    return true;
}

inline bool parsePort(const std::string& s, uint16_t& out) {
    if (s.empty() || s.size() > 5) return false;
    uint32_t v = 0;
    for (char c : s) {
        if (c < '0' || c > '9') return false;
        v = v * 10 + static_cast<uint32_t>(c - '0');
    }
    if (v == 0 || v > 65535) return false;
    out = static_cast<uint16_t>(v);
    return true;
}

/// Parse the request file's text. `server` and `pin` are required; a
/// malformed value fails the whole request rather than connecting somewhere
/// unintended. Unknown keys are ignored.
inline bool parseLaunchRequest(const std::string& text, LaunchRequest& out) {
    LaunchRequest r;
    bool haveServer = false;
    bool havePin = false;
    size_t start = 0;
    while (start <= text.size()) {
        size_t end = text.find('\n', start);
        if (end == std::string::npos) end = text.size();
        std::string line = text.substr(start, end - start);
        if (!line.empty() && line.back() == '\r') line.pop_back();
        const size_t eq = line.find('=');
        if (eq != std::string::npos) {
            const std::string key = line.substr(0, eq);
            const std::string value = line.substr(eq + 1);
            if (key == "server") {
                if (!fvp_session::parse_server_endpoint(value, r.server)) return false;
                haveServer = true;
            } else if (key == "pin") {
                if (!parsePin(value, r.pin)) return false;
                havePin = true;
            } else if (key == "udp_port") {
                // The engine uses base+1..base+3 for video, tracking, audio.
                if (!parsePort(value, r.udpBasePort) || r.udpBasePort > 65532) return false;
            }
        }
        start = end + 1;
    }
    if (!haveServer || !havePin) return false;
    out = r;
    return true;
}

}  // namespace fvp_launch
