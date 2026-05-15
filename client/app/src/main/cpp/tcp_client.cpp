#include "tcp_client.h"
#include "xr_utils.h"

#include <sys/socket.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <cctype>
#include <cerrno>
#include <cstdio>
#include <cstring>
#include <algorithm>
#include <fstream>

#include <mbedtls/x509_crt.h>

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

    // Server uses an ephemeral self-signed cert each launch, so X.509 chain
    // validation is not meaningful. Identity is enforced by TOFU pinning of the
    // leaf cert SHA-256 in verifyOrPinServerCert() below. We use VERIFY_OPTIONAL
    // (not VERIFY_NONE) so mbedtls still parses and stores the peer cert, which
    // we then read via mbedtls_ssl_get_peer_cert().
    mbedtls_ssl_conf_authmode(&m_sslConf, MBEDTLS_SSL_VERIFY_OPTIONAL);
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

    LOGI("TLS handshake complete (cipher: %s)", mbedtls_ssl_get_ciphersuite(&m_ssl));

    if (!verifyOrPinServerCert()) {
        return false;
    }

    m_tlsEnabled = true;
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

    // TLS + TOFU pinning is mandatory. No plaintext fallback: a downgrade would
    // expose the pairing PIN and CONFIG_UPDATE messages to anyone on the LAN.
    if (!initTls()) {
        LOGE("TLS / pinning failed — refusing to connect to %s:%d", serverAddress, port);
        // shutdownTls() invokes mbedtls_net_free() which closes m_netCtx.fd
        // (the same fd as m_socket). Avoid a double close — clear m_socket
        // without calling close() ourselves.
        shutdownTls();
        m_socket = -1;
        return false;
    }

    m_connected = true;
    LOGI("TCP connected to %s:%d (TLS: yes, pinned)", serverAddress, port);
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

// ---------------------------------------------------------------------------
// TOFU certificate pinning
// ---------------------------------------------------------------------------

static std::string sha256Hex(const unsigned char* data, size_t len) {
    unsigned char hash[32];
    // 0 == SHA-256 (not SHA-224)
    if (mbedtls_sha256(data, len, hash, 0) != 0) {
        return {};
    }
    static const char hex[] = "0123456789abcdef";
    std::string out(64, '\0');
    for (size_t i = 0; i < 32; ++i) {
        out[2 * i]     = hex[(hash[i] >> 4) & 0xF];
        out[2 * i + 1] = hex[hash[i] & 0xF];
    }
    return out;
}

static std::string trim(const std::string& s) {
    auto begin = std::find_if_not(s.begin(), s.end(), [](unsigned char c) { return std::isspace(c); });
    auto end = std::find_if_not(s.rbegin(), s.rend(), [](unsigned char c) { return std::isspace(c); }).base();
    return (begin < end) ? std::string(begin, end) : std::string();
}

bool TcpControlClient::verifyOrPinServerCert() {
    if (m_fingerprintStorePath.empty()) {
        LOGE("TOFU: fingerprint store path not configured — refusing connection. "
             "Caller must call setFingerprintStorePath() before connect().");
        return false;
    }

    const mbedtls_x509_crt* cert = mbedtls_ssl_get_peer_cert(&m_ssl);
    if (!cert) {
        LOGE("TOFU: server presented no certificate — refusing connection");
        return false;
    }

    std::string actual = sha256Hex(cert->raw.p, cert->raw.len);
    if (actual.empty()) {
        LOGE("TOFU: SHA-256 of peer cert failed — refusing connection");
        return false;
    }

    // Load pinned fingerprint from disk (cache in member after first read).
    if (m_pinnedFingerprint.empty()) {
        std::ifstream in(m_fingerprintStorePath);
        if (in.good()) {
            std::string line;
            std::getline(in, line);
            m_pinnedFingerprint = trim(line);
        }
    }

    if (m_pinnedFingerprint.empty()) {
        // First connect: pin this cert.
        std::ofstream out(m_fingerprintStorePath, std::ios::trunc);
        if (!out.good()) {
            LOGE("TOFU: cannot write fingerprint to %s — refusing connection",
                 m_fingerprintStorePath.c_str());
            return false;
        }
        out << actual << '\n';
        out.close();
        if (!out.good()) {
            LOGE("TOFU: write to %s failed — refusing connection",
                 m_fingerprintStorePath.c_str());
            return false;
        }
        m_pinnedFingerprint = actual;
        LOGI("TOFU: pinned new server cert (sha256=%s)", actual.c_str());
        return true;
    }

    // Constant-time-ish comparison on equal-length hex strings.
    if (m_pinnedFingerprint.size() != actual.size()) {
        LOGE("TOFU: pinned fingerprint length mismatch (stored=%zu, actual=%zu) — "
             "refusing connection. Delete %s to re-pair.",
             m_pinnedFingerprint.size(), actual.size(), m_fingerprintStorePath.c_str());
        return false;
    }
    unsigned char diff = 0;
    for (size_t i = 0; i < actual.size(); ++i) {
        diff |= (unsigned char)(m_pinnedFingerprint[i] ^ actual[i]);
    }
    if (diff != 0) {
        LOGE("TOFU: server cert fingerprint MISMATCH — possible MITM. "
             "Expected %s, got %s. Refusing connection. "
             "If you intentionally re-paired with a new server, delete %s.",
             m_pinnedFingerprint.c_str(), actual.c_str(),
             m_fingerprintStorePath.c_str());
        return false;
    }

    LOGI("TOFU: server cert matches pinned fingerprint");
    return true;
}

void TcpControlClient::clearPinnedFingerprint() {
    m_pinnedFingerprint.clear();
    if (!m_fingerprintStorePath.empty()) {
        if (std::remove(m_fingerprintStorePath.c_str()) == 0) {
            LOGI("TOFU: cleared pinned fingerprint at %s",
                 m_fingerprintStorePath.c_str());
        }
    }
}
