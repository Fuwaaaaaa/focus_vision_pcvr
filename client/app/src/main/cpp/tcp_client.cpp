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

bool TcpControlClient::initTls() {
    mbedtls_ssl_init(&m_ssl);
    mbedtls_ssl_config_init(&m_sslConf);
    mbedtls_entropy_init(&m_entropy);
    mbedtls_ctr_drbg_init(&m_ctrDrbg);
    mbedtls_net_init(&m_netCtx);

    if (mbedtls_ctr_drbg_seed(&m_ctrDrbg, mbedtls_entropy_func, &m_entropy, nullptr, 0) != 0) {
        LOGE("MbedTLS: ctr_drbg_seed failed");
        return false;
    }

    if (mbedtls_ssl_config_defaults(&m_sslConf, MBEDTLS_SSL_IS_CLIENT,
            MBEDTLS_SSL_TRANSPORT_STREAM, MBEDTLS_SSL_PRESET_DEFAULT) != 0) {
        LOGE("MbedTLS: ssl_config_defaults failed");
        return false;
    }

    // Skip server certificate verification (self-signed cert, TOFU model)
    mbedtls_ssl_conf_authmode(&m_sslConf, MBEDTLS_SSL_VERIFY_NONE);
    mbedtls_ssl_conf_rng(&m_sslConf, mbedtls_ctr_drbg_random, &m_ctrDrbg);

    if (mbedtls_ssl_setup(&m_ssl, &m_sslConf) != 0) {
        LOGE("MbedTLS: ssl_setup failed");
        return false;
    }

    // Set the file descriptor for MbedTLS I/O
    m_netCtx.fd = m_socket;
    mbedtls_ssl_set_bio(&m_ssl, &m_netCtx, mbedtls_net_send, mbedtls_net_recv, nullptr);

    // Perform TLS handshake
    int ret;
    while ((ret = mbedtls_ssl_handshake(&m_ssl)) != 0) {
        if (ret != MBEDTLS_ERR_SSL_WANT_READ && ret != MBEDTLS_ERR_SSL_WANT_WRITE) {
            LOGE("MbedTLS: handshake failed: -0x%04x", -ret);
            return false;
        }
    }

    m_tlsEnabled = true;
    LOGI("TLS handshake complete (cipher: %s)", mbedtls_ssl_get_ciphersuite(&m_ssl));
    return true;
}

void TcpControlClient::shutdownTls() {
    if (m_tlsEnabled) {
        mbedtls_ssl_close_notify(&m_ssl);
        m_tlsEnabled = false;
    }
    mbedtls_ssl_free(&m_ssl);
    mbedtls_ssl_config_free(&m_sslConf);
    mbedtls_ctr_drbg_free(&m_ctrDrbg);
    mbedtls_entropy_free(&m_entropy);
    mbedtls_net_free(&m_netCtx);
}

int TcpControlClient::tlsSend(const uint8_t* data, int len) {
    return mbedtls_ssl_write(&m_ssl, data, len);
}

int TcpControlClient::tlsRecv(uint8_t* data, int len) {
    return mbedtls_ssl_read(&m_ssl, data, len);
}

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

    // Attempt TLS handshake
    if (!initTls()) {
        LOGW("TLS handshake failed, falling back to plaintext");
    }

    m_connected = true;
    LOGI("TCP connected to %s:%d (TLS: %s)", serverAddress, port,
         m_tlsEnabled ? "yes" : "no");
    return true;
}

void TcpControlClient::disconnect() {
    shutdownTls();
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

    uint32_t len = 1 + payloadLen;

    auto writeAll = [&](const void* data, int size) -> bool {
        if (m_tlsEnabled) {
            return tlsSend((const uint8_t*)data, size) == size;
        } else {
            return send(m_socket, data, size, 0) == size;
        }
    };

    if (!writeAll(&len, 4)) return false;
    if (!writeAll(&type, 1)) return false;
    if (payloadLen > 0 && payload) {
        if (!writeAll(payload, payloadLen)) return false;
    }
    return true;
}

bool TcpControlClient::recvMessage(uint8_t& outType, std::vector<uint8_t>& outPayload) {
    if (m_socket < 0) return false;

    auto readAll = [&](void* data, int size) -> bool {
        int total = 0;
        while (total < size) {
            int n;
            if (m_tlsEnabled) {
                n = tlsRecv((uint8_t*)data + total, size - total);
            } else {
                n = recv(m_socket, (uint8_t*)data + total, size - total, 0);
            }
            if (n <= 0) return false;
            total += n;
        }
        return true;
    };

    uint32_t len = 0;
    if (!readAll(&len, 4)) return false;
    if (len == 0 || len > 65536) return false;

    std::vector<uint8_t> buf(len);
    if (!readAll(buf.data(), len)) return false;

    outType = buf[0];
    outPayload.assign(buf.begin() + 1, buf.end());
    return true;
}
