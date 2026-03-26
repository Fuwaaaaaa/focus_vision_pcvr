#pragma once

#include <cstdint>
#include <atomic>

/// UDP packet receiver. Stub in Step 4, implemented in Step 5.
class NetworkReceiver {
public:
    bool init(const char* bindAddress, int port);
    void shutdown();

    /// Receive a packet into buffer. Returns bytes received, or -1 on error.
    /// Non-blocking when no data available.
    int receive(uint8_t* buffer, int maxSize);

    bool isInitialized() const { return m_initialized; }

private:
    int m_socket = -1;
    std::atomic<bool> m_initialized{false};
};
