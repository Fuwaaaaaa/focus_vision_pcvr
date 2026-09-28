#include "net_compat.h" // first: winsock2.h must precede anything that pulls in windows.h
#include "tcp_client.h"
#include "client_protocol.h"
#include "xr_utils.h"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <fstream>

#include <mbedtls/sha256.h>
#include <mbedtls/x509_crt.h>
#include <psa/crypto.h>

namespace {

namespace proto = fvp_client_protocol;
using Clock = std::chrono::steady_clock;

/// A write that cannot make progress for this long means the server stopped
/// reading; the connection is treated as dead.
constexpr int kWriteTimeoutMs = 3000;

int msLeft(Clock::time_point deadline) {
    const auto left = std::chrono::duration_cast<std::chrono::milliseconds>(deadline - Clock::now());
    return left.count() > 0 ? static_cast<int>(left.count()) : 0;
}

bool isRetry(int ret) {
    return ret == MBEDTLS_ERR_SSL_WANT_READ || ret == MBEDTLS_ERR_SSL_WANT_WRITE;
}

}  // namespace

TcpControlClient::TcpControlClient() {
    initContexts();
}

TcpControlClient::~TcpControlClient() {
    disconnect();
    freeContexts();
}

void TcpControlClient::initContexts() {
    mbedtls_net_init(&m_netCtx);
    mbedtls_ssl_init(&m_ssl);
    mbedtls_ssl_config_init(&m_sslConf);
    mbedtls_entropy_init(&m_entropy);
    mbedtls_ctr_drbg_init(&m_ctrDrbg);
}

void TcpControlClient::freeContexts() {
    mbedtls_net_free(&m_netCtx); // closes the socket, if open
    mbedtls_ssl_free(&m_ssl);
    mbedtls_ssl_config_free(&m_sslConf);
    mbedtls_ctr_drbg_free(&m_ctrDrbg);
    mbedtls_entropy_free(&m_entropy);
}

bool TcpControlClient::connect(const char* serverAddress, int port, int timeoutMs) {
    disconnect();

    // TLS 1.3 in MbedTLS 3.x runs on PSA crypto, which must be initialised
    // first (repeated calls are no-ops).
    if (psa_crypto_init() != PSA_SUCCESS) {
        LOGE("psa_crypto_init failed");
        return false;
    }

    const fvp_net::Socket sock =
        fvp_net::connectTcp(serverAddress, static_cast<uint16_t>(port), timeoutMs);
    if (sock == fvp_net::kInvalidSocket) {
        LOGE("TCP connect to %s:%d failed", serverAddress, port);
        return false;
    }
    // From here the socket belongs to m_netCtx: mbedtls_net_free() closes it.
    m_netCtx.fd = static_cast<int>(sock);
    // Disable Nagle's algorithm for low-latency control messages.
    fvp_net::setIntOption(sock, IPPROTO_TCP, TCP_NODELAY, 1);

    if (!tlsHandshake(timeoutMs) || !checkServerCert()) {
        // TLS + pinning is mandatory. No plaintext fallback: a downgrade would
        // expose the pairing PIN and CONFIG_UPDATE messages to anyone on the LAN.
        LOGE("TLS / pinning failed — refusing to connect to %s:%d", serverAddress, port);
        disconnect();
        return false;
    }

    m_rxBuf.clear();
    m_connected = true;
    LOGI("TCP connected to %s:%d (TLS: %s, cipher %s)", serverAddress, port,
         mbedtls_ssl_get_version(&m_ssl), mbedtls_ssl_get_ciphersuite(&m_ssl));
    return true;
}

bool TcpControlClient::tlsHandshake(int timeoutMs) {
    if (mbedtls_ctr_drbg_seed(&m_ctrDrbg, mbedtls_entropy_func, &m_entropy, nullptr, 0) != 0) {
        LOGE("MbedTLS: ctr_drbg_seed failed");
        return false;
    }
    if (mbedtls_ssl_config_defaults(&m_sslConf, MBEDTLS_SSL_IS_CLIENT,
            MBEDTLS_SSL_TRANSPORT_STREAM, MBEDTLS_SSL_PRESET_DEFAULT) != 0) {
        LOGE("MbedTLS: ssl_config_defaults failed");
        return false;
    }
    // The server's certificate is self-signed, so chain validation means
    // nothing; the server is authenticated by the TOFU fingerprint check in
    // checkServerCert(). VERIFY_OPTIONAL (not NONE) keeps the peer
    // certificate available to mbedtls_ssl_get_peer_cert().
    mbedtls_ssl_conf_authmode(&m_sslConf, MBEDTLS_SSL_VERIFY_OPTIONAL);
    mbedtls_ssl_conf_rng(&m_sslConf, mbedtls_ctr_drbg_random, &m_ctrDrbg);

    if (mbedtls_ssl_setup(&m_ssl, &m_sslConf) != 0) {
        LOGE("MbedTLS: ssl_setup failed");
        return false;
    }
    // The engine's certificate names "localhost". Newer MbedTLS releases
    // refuse to verify a certificate without a hostname set.
    mbedtls_ssl_set_hostname(&m_ssl, "localhost");

    // Non-blocking socket, and poll it whenever MbedTLS asks for more data:
    // the documented way to bound TLS I/O by a timeout.
    mbedtls_net_set_nonblock(&m_netCtx);
    mbedtls_ssl_set_bio(&m_ssl, &m_netCtx, mbedtls_net_send, mbedtls_net_recv, nullptr);

    const auto deadline = Clock::now() + std::chrono::milliseconds(timeoutMs);
    for (;;) {
        const int ret = mbedtls_ssl_handshake(&m_ssl);
        if (ret == 0) break;
        if (!isRetry(ret)) {
            LOGE("MbedTLS: handshake failed: -0x%04x", -ret);
            return false;
        }
        const int left = msLeft(deadline);
        if (left == 0) {
            LOGE("MbedTLS: handshake timed out after %d ms", timeoutMs);
            return false;
        }
        const uint32_t dir = ret == MBEDTLS_ERR_SSL_WANT_READ ? MBEDTLS_NET_POLL_READ
                                                              : MBEDTLS_NET_POLL_WRITE;
        if (mbedtls_net_poll(&m_netCtx, dir, static_cast<uint32_t>(left)) < 0) {
            LOGE("MbedTLS: poll failed during handshake");
            return false;
        }
    }
    m_tlsEstablished = true;
    return true;
}

void TcpControlClient::disconnect() {
    if (m_tlsEstablished) {
        mbedtls_ssl_close_notify(&m_ssl); // best effort; the socket is non-blocking
        m_tlsEstablished = false;
    }
    // Free and re-init everything so the next connect() starts clean. Every
    // context was initialised in the constructor, so freeing is always safe,
    // and mbedtls_net_free() owns (and closes) the socket — nothing else does.
    freeContexts();
    initContexts();
    m_rxBuf.clear();
    m_peerNeedsPinning = false;
    m_connected = false;
}

TcpControlClient::HandshakeResult TcpControlClient::handshake(uint32_t pin, int stepTimeoutMs) {
    std::vector<uint8_t> payload;

    // HELLO: protocol version (u16 LE) + capability byte.
    auto hello = proto::buildHelloPayload(proto::PROTOCOL_VERSION,
                                          proto::hello_caps::RESOLUTION_SCALE);
    if (!sendMessage(proto::msg::HELLO, hello.data(), static_cast<int>(hello.size()))) {
        return HandshakeResult::Failed;
    }
    if (!expectMessage(proto::msg::HELLO_ACK, payload, stepTimeoutMs)) return HandshakeResult::Failed;
    if (!expectMessage(proto::msg::PIN_REQUEST, payload, stepTimeoutMs)) return HandshakeResult::Failed;

    // PIN_RESPONSE: the 6-digit PIN as u32 LE.
    uint8_t pinBytes[4];
    proto::writeU32Le(pinBytes, pin);
    if (!sendMessage(proto::msg::PIN_RESPONSE, pinBytes, 4)) return HandshakeResult::Failed;

    if (!expectMessage(proto::msg::PIN_RESULT, payload, stepTimeoutMs)) return HandshakeResult::Failed;
    if (payload.empty() || payload[0] != 0x01) {
        LOGE("PIN rejected");
        return HandshakeResult::PinRejected;
    }
    LOGI("PIN accepted");

    if (!expectMessage(proto::msg::STREAM_CONFIG, payload, stepTimeoutMs)) return HandshakeResult::Failed;
    proto::StreamConfigView cfg;
    if (!proto::parseStreamConfig(payload.data(), payload.size(), cfg)) {
        LOGE("STREAM_CONFIG too short (%zu bytes)", payload.size());
        return HandshakeResult::Failed;
    }
    m_config.width = cfg.width;
    m_config.height = cfg.height;
    m_config.bitrateMbps = cfg.bitrateMbps;
    m_config.framerate = cfg.framerate;
    m_config.codec = cfg.codec;
    m_config.encodedWidth = cfg.encodedWidth;
    m_config.encodedHeight = cfg.encodedHeight;
    LOGI("Stream config: native %ux%u, encoded %ux%u @ %u Mbps, %u fps, codec=%u",
        m_config.width, m_config.height, m_config.encodedWidth, m_config.encodedHeight,
        m_config.bitrateMbps, m_config.framerate, m_config.codec);

    // Paired: this server is the one to trust from now on.
    if (!pinPeerFingerprint()) return HandshakeResult::Failed;
    return HandshakeResult::Ok;
}

bool TcpControlClient::startStream() {
    if (!sendMessage(proto::msg::STREAM_START, nullptr, 0)) return false;
    LOGI("Handshake complete, ready to stream");
    return true;
}

bool TcpControlClient::requestIdr() {
    return sendMessage(proto::msg::IDR_REQUEST, nullptr, 0);
}

bool TcpControlClient::sendMessage(uint8_t type, const uint8_t* payload, int payloadLen) {
    if (!m_tlsEstablished || payloadLen < 0) return false;
    // One buffer, one TLS record: [length u32 LE][type][payload].
    std::vector<uint8_t> frame(5 + static_cast<size_t>(payloadLen));
    proto::writeU32Le(frame.data(), static_cast<uint32_t>(1 + payloadLen));
    frame[4] = type;
    if (payloadLen > 0 && payload) {
        std::memcpy(frame.data() + 5, payload, static_cast<size_t>(payloadLen));
    }
    return writeAll(frame.data(), frame.size());
}

bool TcpControlClient::writeAll(const uint8_t* data, size_t len) {
    const auto deadline = Clock::now() + std::chrono::milliseconds(kWriteTimeoutMs);
    size_t off = 0;
    while (off < len) {
        const int ret = mbedtls_ssl_write(&m_ssl, data + off, len - off);
        if (ret > 0) {
            off += static_cast<size_t>(ret);
            continue;
        }
        const int left = msLeft(deadline);
        if (!isRetry(ret) || left == 0) {
            LOGE("TLS write failed: -0x%04x", -ret);
            m_connected = false;
            return false;
        }
        const uint32_t dir = ret == MBEDTLS_ERR_SSL_WANT_READ ? MBEDTLS_NET_POLL_READ
                                                              : MBEDTLS_NET_POLL_WRITE;
        if (mbedtls_net_poll(&m_netCtx, dir, static_cast<uint32_t>(left)) < 0) {
            m_connected = false;
            return false;
        }
    }
    return true;
}

int TcpControlClient::takeMessage(uint8_t& outType, std::vector<uint8_t>& outPayload) {
    if (m_rxBuf.size() < 4) return 0;
    const uint32_t len = proto::readU32Le(m_rxBuf.data());
    if (len == 0 || len > proto::MAX_CONTROL_MESSAGE_LEN) {
        LOGE("Bad control message length %u", len);
        return -1;
    }
    if (m_rxBuf.size() < 4 + static_cast<size_t>(len)) return 0;
    outType = m_rxBuf[4];
    outPayload.assign(m_rxBuf.begin() + 5, m_rxBuf.begin() + 4 + len);
    m_rxBuf.erase(m_rxBuf.begin(), m_rxBuf.begin() + 4 + len);
    return 1;
}

TcpControlClient::RecvResult TcpControlClient::recvMessage(
        uint8_t& outType, std::vector<uint8_t>& outPayload, int timeoutMs) {
    if (!m_tlsEstablished) return RecvResult::Closed;
    const auto deadline = Clock::now() + std::chrono::milliseconds(timeoutMs);
    uint8_t buf[4096];
    for (;;) {
        const int framed = takeMessage(outType, outPayload);
        if (framed > 0) return RecvResult::Message;
        if (framed < 0) {
            m_connected = false;
            return RecvResult::Closed;
        }

        const int ret = mbedtls_ssl_read(&m_ssl, buf, sizeof(buf));
        if (ret > 0) {
            m_rxBuf.insert(m_rxBuf.end(), buf, buf + ret);
            continue;
        }
#ifdef MBEDTLS_ERR_SSL_RECEIVED_NEW_SESSION_TICKET
        // A TLS 1.3 session ticket, not application data: read on.
        if (ret == MBEDTLS_ERR_SSL_RECEIVED_NEW_SESSION_TICKET) continue;
#endif
        if (!isRetry(ret)) {
            // 0 = EOF, PEER_CLOSE_NOTIFY, or an error: the connection is gone.
            if (ret != 0 && ret != MBEDTLS_ERR_SSL_PEER_CLOSE_NOTIFY) {
                LOGE("TLS read failed: -0x%04x", -ret);
            }
            m_connected = false;
            return RecvResult::Closed;
        }
        const int left = msLeft(deadline);
        if (left == 0) return RecvResult::Timeout;
        const uint32_t dir = ret == MBEDTLS_ERR_SSL_WANT_READ ? MBEDTLS_NET_POLL_READ
                                                              : MBEDTLS_NET_POLL_WRITE;
        if (mbedtls_net_poll(&m_netCtx, dir, static_cast<uint32_t>(left)) < 0) {
            m_connected = false;
            return RecvResult::Closed;
        }
    }
}

bool TcpControlClient::expectMessage(uint8_t type, std::vector<uint8_t>& payload, int timeoutMs) {
    uint8_t got = 0;
    const RecvResult r = recvMessage(got, payload, timeoutMs);
    if (r != RecvResult::Message) {
        LOGE("Expected message 0x%02x: %s", type,
             r == RecvResult::Timeout ? "timed out" : "connection closed");
        return false;
    }
    if (got != type) {
        LOGE("Expected message 0x%02x, got 0x%02x", type, got);
        return false;
    }
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

bool TcpControlClient::checkServerCert() {
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

    const std::string actual = sha256Hex(cert->raw.p, cert->raw.len);
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

    m_peerFingerprint = actual;
    if (m_pinnedFingerprint.empty()) {
        // First server: pinned by handshake() once pairing with it succeeds.
        m_peerNeedsPinning = true;
        LOGI("TOFU: no pinned server yet (sha256=%s)", actual.c_str());
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

bool TcpControlClient::pinPeerFingerprint() {
    if (!m_peerNeedsPinning) return true;
    std::ofstream out(m_fingerprintStorePath, std::ios::trunc);
    if (!out.good()) {
        LOGE("TOFU: cannot write fingerprint to %s — refusing connection",
             m_fingerprintStorePath.c_str());
        return false;
    }
    out << m_peerFingerprint << '\n';
    out.close();
    if (!out.good()) {
        LOGE("TOFU: write to %s failed — refusing connection", m_fingerprintStorePath.c_str());
        return false;
    }
    m_pinnedFingerprint = m_peerFingerprint;
    m_peerNeedsPinning = false;
    LOGI("TOFU: pinned server cert (sha256=%s)", m_peerFingerprint.c_str());
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
