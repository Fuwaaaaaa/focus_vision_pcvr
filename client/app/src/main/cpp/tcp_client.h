#pragma once

#include <cstdint>
#include <string>
#include <vector>
#include <atomic>

/// TCP control channel client. Handles handshake, PIN pairing, stream config.
class TcpControlClient {
public:
    struct StreamConfig {
        uint32_t width = 0;
        uint32_t height = 0;
        uint32_t bitrateMbps = 0;
        uint32_t framerate = 0;
        uint8_t codec = 1; // 0=H264, 1=H265
    };

    bool connect(const char* serverAddress, int port);
    void disconnect();

    /// Run the handshake: HELLO → PIN → CONFIG → START.
    /// Returns true if streaming is ready.
    bool handshake(uint16_t pin);

    const StreamConfig& getStreamConfig() const { return m_config; }
    bool isConnected() const { return m_connected; }

    /// Request an IDR keyframe from the server (msg_type 0x30).
    bool requestIdr();

    /// Send a framed message: [length:u32 LE][type:u8][payload]
    bool sendMessage(uint8_t type, const uint8_t* payload, int payloadLen);

    /// Receive a framed message. Returns (type, payload). Blocking.
    bool recvMessage(uint8_t& outType, std::vector<uint8_t>& outPayload);

private:
    int m_socket = -1;
    std::atomic<bool> m_connected{false};
    StreamConfig m_config;
};
