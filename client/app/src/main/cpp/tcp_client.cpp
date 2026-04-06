#include "tcp_client.h"
#include "xr_utils.h"

#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <cerrno>
#include <cstring>

// Protocol message types (must match Rust side)
static constexpr uint8_t MSG_HELLO = 0x01;
static constexpr uint8_t MSG_HELLO_ACK = 0x02;
static constexpr uint8_t MSG_PIN_REQUEST = 0x03;
static constexpr uint8_t MSG_PIN_RESPONSE = 0x04;
static constexpr uint8_t MSG_PIN_RESULT = 0x05;
static constexpr uint8_t MSG_STREAM_CONFIG = 0x06;
static constexpr uint8_t MSG_STREAM_START = 0x07;
static constexpr uint8_t MSG_IDR_REQUEST = 0x30;

bool TcpControlClient::connect(const char* serverAddress, int port) {
    m_socket = socket(AF_INET, SOCK_STREAM, 0);
    if (m_socket < 0) {
        LOGE("Failed to create TCP socket: %s", strerror(errno));
        return false;
    }

    // Disable Nagle's algorithm for low-latency control messages
    int flag = 1;
    setsockopt(m_socket, IPPROTO_TCP, TCP_NODELAY, &flag, sizeof(flag));

    struct sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    inet_pton(AF_INET, serverAddress, &addr.sin_addr);

    if (::connect(m_socket, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        LOGE("TCP connect to %s:%d failed: %s", serverAddress, port, strerror(errno));
        close(m_socket);
        m_socket = -1;
        return false;
    }

    m_connected = true;
    LOGI("TCP connected to %s:%d", serverAddress, port);
    return true;
}

void TcpControlClient::disconnect() {
    if (m_socket >= 0) {
        close(m_socket);
        m_socket = -1;
    }
    m_connected = false;
}

bool TcpControlClient::handshake(uint32_t pin) {
    uint8_t type;
    std::vector<uint8_t> payload;

    // Step 1: Send HELLO
    uint8_t version[] = {1, 0}; // v1.0
    if (!sendMessage(MSG_HELLO, version, 2)) return false;

    // Step 2: Receive HELLO_ACK
    if (!recvMessage(type, payload) || type != MSG_HELLO_ACK) {
        LOGE("Expected HELLO_ACK, got %d", type);
        return false;
    }

    // Step 3: Receive PIN_REQUEST
    if (!recvMessage(type, payload) || type != MSG_PIN_REQUEST) {
        LOGE("Expected PIN_REQUEST, got %d", type);
        return false;
    }

    // Step 4: Send PIN_RESPONSE (6-digit PIN as u32 LE)
    uint8_t pinBytes[4];
    memcpy(pinBytes, &pin, 4);
    if (!sendMessage(MSG_PIN_RESPONSE, pinBytes, 4)) return false;

    // Step 5: Receive PIN_RESULT
    if (!recvMessage(type, payload) || type != MSG_PIN_RESULT) {
        LOGE("Expected PIN_RESULT, got %d", type);
        return false;
    }
    if (payload.empty() || payload[0] != 0x01) {
        LOGE("PIN rejected");
        return false;
    }
    LOGI("PIN accepted");

    // Step 6: Receive STREAM_CONFIG
    if (!recvMessage(type, payload) || type != MSG_STREAM_CONFIG) {
        LOGE("Expected STREAM_CONFIG, got %d", type);
        return false;
    }
    if (payload.size() >= 17) {
        memcpy(&m_config.width, &payload[0], 4);
        memcpy(&m_config.height, &payload[4], 4);
        memcpy(&m_config.bitrateMbps, &payload[8], 4);
        memcpy(&m_config.framerate, &payload[12], 4);
        m_config.codec = payload[16];
        LOGI("Stream config: %ux%u @ %u Mbps, %u fps, codec=%u",
            m_config.width, m_config.height, m_config.bitrateMbps,
            m_config.framerate, m_config.codec);
    }

    // Step 7: Send STREAM_START
    if (!sendMessage(MSG_STREAM_START, nullptr, 0)) return false;

    LOGI("Handshake complete, ready to stream");
    return true;
}

bool TcpControlClient::requestIdr() {
    LOGI("Sending IDR_REQUEST to server");
    return sendMessage(MSG_IDR_REQUEST, nullptr, 0);
}

bool TcpControlClient::sendMessage(uint8_t type, const uint8_t* payload, int payloadLen) {
    if (m_socket < 0) return false;

    uint32_t len = 1 + payloadLen; // type byte + payload
    // Send length (LE)
    if (send(m_socket, &len, 4, 0) != 4) return false;
    // Send type
    if (send(m_socket, &type, 1, 0) != 1) return false;
    // Send payload
    if (payloadLen > 0 && payload) {
        if (send(m_socket, payload, payloadLen, 0) != payloadLen) return false;
    }
    return true;
}

bool TcpControlClient::recvMessage(uint8_t& outType, std::vector<uint8_t>& outPayload) {
    if (m_socket < 0) return false;

    // Read length
    uint32_t len = 0;
    if (recv(m_socket, &len, 4, MSG_WAITALL) != 4) return false;
    if (len == 0 || len > 65536) return false;

    // Read message
    std::vector<uint8_t> buf(len);
    ssize_t received = recv(m_socket, buf.data(), len, MSG_WAITALL);
    if (received != (ssize_t)len) return false;

    outType = buf[0];
    outPayload.assign(buf.begin() + 1, buf.end());
    return true;
}
