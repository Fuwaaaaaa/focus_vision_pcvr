#include "network_receiver.h"
#include "xr_utils.h"

#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <fcntl.h>
#include <cerrno>
#include <cstring>

bool NetworkReceiver::init(const char* bindAddress, int port) {
    m_socket = socket(AF_INET, SOCK_DGRAM, 0);
    if (m_socket < 0) {
        LOGE("Failed to create UDP socket: %s", strerror(errno));
        return false;
    }

    // Set non-blocking
    int flags = fcntl(m_socket, F_GETFL, 0);
    fcntl(m_socket, F_SETFL, flags | O_NONBLOCK);

    // Set receive buffer size (2MB for video packets)
    int bufSize = 2 * 1024 * 1024;
    setsockopt(m_socket, SOL_SOCKET, SO_RCVBUF, &bufSize, sizeof(bufSize));

    struct sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port);
    inet_pton(AF_INET, bindAddress, &addr.sin_addr);

    if (bind(m_socket, (struct sockaddr*)&addr, sizeof(addr)) < 0) {
        LOGE("Failed to bind UDP socket to %s:%d: %s", bindAddress, port, strerror(errno));
        close(m_socket);
        m_socket = -1;
        return false;
    }

    m_initialized = true;
    LOGI("UDP receiver bound to %s:%d", bindAddress, port);
    return true;
}

void NetworkReceiver::shutdown() {
    if (m_socket >= 0) {
        close(m_socket);
        m_socket = -1;
    }
    m_initialized = false;
}

int NetworkReceiver::receive(uint8_t* buffer, int maxSize) {
    if (!m_initialized || m_socket < 0) return -1;

    ssize_t received = recv(m_socket, buffer, maxSize, 0);
    if (received < 0) {
        if (errno == EAGAIN || errno == EWOULDBLOCK) {
            return 0; // No data available (non-blocking)
        }
        return -1;
    }
    return (int)received;
}
