#include "tracking_sender.h"
#include "xr_utils.h"

#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <cstring>
#include <cerrno>

static constexpr uint8_t PACKET_HEAD_POSE = 0x01;
static constexpr uint8_t PACKET_CONTROLLER = 0x02;

// Helper: append bytes to buffer
static void appendF32(uint8_t* buf, int& offset, float v) {
    memcpy(buf + offset, &v, 4); offset += 4;
}
static void appendU64(uint8_t* buf, int& offset, uint64_t v) {
    memcpy(buf + offset, &v, 8); offset += 8;
}
static void appendU32(uint8_t* buf, int& offset, uint32_t v) {
    memcpy(buf + offset, &v, 4); offset += 4;
}

bool TrackingSender::init(const char* serverAddress, int port) {
    m_socket = socket(AF_INET, SOCK_DGRAM, 0);
    if (m_socket < 0) {
        LOGE("Failed to create tracking UDP socket: %s", strerror(errno));
        return false;
    }

    memset(&m_serverAddr, 0, sizeof(m_serverAddr));
    m_serverAddr.sin_family = AF_INET;
    m_serverAddr.sin_port = htons(port);
    inet_pton(AF_INET, serverAddress, &m_serverAddr.sin_addr);

    m_initialized = true;
    LOGI("Tracking sender initialized → %s:%d", serverAddress, port);
    return true;
}

void TrackingSender::shutdown() {
    if (m_socket >= 0) {
        close(m_socket);
        m_socket = -1;
    }
    m_initialized = false;
}

void TrackingSender::sendHeadPose(const XrPosef& pose, int64_t timestampNs,
                                  float gazeX, float gazeY, bool gazeValid) {
    if (!m_initialized) return;

    // Packet: [type:1B][timestamp:8B][pos:12B][orient:16B][gaze_x:4B][gaze_y:4B][gaze_valid:1B] = 46B
    uint8_t buf[46];
    int off = 0;

    buf[off++] = PACKET_HEAD_POSE;
    appendU64(buf, off, (uint64_t)timestampNs);
    appendF32(buf, off, pose.position.x);
    appendF32(buf, off, pose.position.y);
    appendF32(buf, off, pose.position.z);
    appendF32(buf, off, pose.orientation.x);
    appendF32(buf, off, pose.orientation.y);
    appendF32(buf, off, pose.orientation.z);
    appendF32(buf, off, pose.orientation.w);
    appendF32(buf, off, gazeX);
    appendF32(buf, off, gazeY);
    buf[off++] = gazeValid ? 1 : 0;

    sendto(m_socket, buf, off, 0,
        (struct sockaddr*)&m_serverAddr, sizeof(m_serverAddr));
}

void TrackingSender::sendControllerState(
    uint8_t controllerId, const XrPosef& pose, int64_t timestampNs,
    float trigger, float grip, float thumbstickX, float thumbstickY,
    uint32_t buttonFlags, uint8_t battery)
{
    if (!m_initialized) return;

    // Packet: [type:1B][id:1B][ts:8B][pos:12B][orient:16B][trigger:4B][grip:4B]
    //         [thumbX:4B][thumbY:4B][buttons:4B][battery:1B] = 59 bytes
    uint8_t buf[64];
    int off = 0;

    buf[off++] = PACKET_CONTROLLER;
    buf[off++] = controllerId;
    appendU64(buf, off, (uint64_t)timestampNs);
    appendF32(buf, off, pose.position.x);
    appendF32(buf, off, pose.position.y);
    appendF32(buf, off, pose.position.z);
    appendF32(buf, off, pose.orientation.x);
    appendF32(buf, off, pose.orientation.y);
    appendF32(buf, off, pose.orientation.z);
    appendF32(buf, off, pose.orientation.w);
    appendF32(buf, off, trigger);
    appendF32(buf, off, grip);
    appendF32(buf, off, thumbstickX);
    appendF32(buf, off, thumbstickY);
    appendU32(buf, off, buttonFlags);
    buf[off++] = battery;

    sendto(m_socket, buf, off, 0,
        (struct sockaddr*)&m_serverAddr, sizeof(m_serverAddr));
}
