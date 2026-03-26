#pragma once

#include <openxr/openxr.h>
#include <cstdint>
#include <atomic>

/// Sends 6DoF head tracking data from HMD to PC via UDP.
/// Packet format: [type:1B][timestamp_ns:8B][position:12B][orientation:16B] = 37 bytes
class TrackingSender {
public:
    bool init(const char* serverAddress, int port);
    void shutdown();

    /// Send the current head pose. Call every frame (~90Hz).
    void sendHeadPose(const XrPosef& pose, int64_t timestampNs);

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
