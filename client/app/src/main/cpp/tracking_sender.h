#pragma once

#include <openxr/openxr.h>
#include <cstdint>
#include <atomic>
#include <netinet/in.h>

/// Sends 6DoF head tracking + gaze data from HMD to PC via UDP.
/// Packet format: [type:1B][timestamp_ns:8B][position:12B][orientation:16B]
///                [gaze_x:4B][gaze_y:4B][gaze_valid:1B] = 46 bytes
class TrackingSender {
public:
    bool init(const char* serverAddress, int port);
    void shutdown();

    /// Send the current head pose + eye gaze. Call every frame (~90Hz).
    void sendHeadPose(const XrPosef& pose, int64_t timestampNs,
                      float gazeX = 0.5f, float gazeY = 0.5f, bool gazeValid = false);

    /// Send controller state. Call at 60Hz+.
    void sendControllerState(uint8_t controllerId, const XrPosef& pose,
                              int64_t timestampNs,
                              float trigger, float grip,
                              float thumbstickX, float thumbstickY,
                              uint32_t buttonFlags, uint8_t battery);

private:
    int m_socket = -1;
    struct sockaddr_in m_serverAddr{};
    std::atomic<bool> m_initialized{false};
};
