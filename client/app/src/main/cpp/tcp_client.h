#pragma once

#include <cstdint>
#include <string>
#include <vector>
#include <atomic>

#include <mbedtls/ssl.h>
#include <mbedtls/entropy.h>
#include <mbedtls/ctr_drbg.h>
#include <mbedtls/net_sockets.h>
#include <mbedtls/sha256.h>

/// TCP control channel client. Handles handshake, PIN pairing, stream config.
///
/// Security model: TLS 1.3 with TOFU (Trust-On-First-Use) certificate pinning.
/// Caller MUST call setFingerprintStorePath() before connect() — without it the
/// client refuses to connect, since accepting any cert without persistence would
/// allow silent MITM substitution between sessions.
class TcpControlClient {
public:
    struct StreamConfig {
        uint32_t width = 0;
        uint32_t height = 0;
        uint32_t bitrateMbps = 0;
        uint32_t framerate = 0;
        uint8_t codec = 1; // 0=H264, 1=H265
    };

    /// Set the file path used to persist the pinned server cert SHA-256.
    /// Recommended location: <ANativeActivity::internalDataPath>/server_fingerprint.hex
    /// Must be called before connect().
    void setFingerprintStorePath(std::string path) { m_fingerprintStorePath = std::move(path); }

    /// Forget the pinned fingerprint (e.g. after the user explicitly re-pairs
    /// with a new server). Removes the on-disk file if present.
    void clearPinnedFingerprint();

    bool connect(const char* serverAddress, int port);
    void disconnect();

    /// Run the handshake: HELLO → PIN → CONFIG → START.
    /// Returns true if streaming is ready.
    bool handshake(uint32_t pin);

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

    // TLS state
    bool m_tlsEnabled = false;
    mbedtls_ssl_context m_ssl;
    mbedtls_ssl_config m_sslConf;
    mbedtls_entropy_context m_entropy;
    mbedtls_ctr_drbg_context m_ctrDrbg;
    mbedtls_net_context m_netCtx;

    // TOFU pinning state.
    std::string m_fingerprintStorePath; // empty == not configured (connect refused)
    std::string m_pinnedFingerprint;    // hex sha256, loaded from disk on first verify

    bool initTls();
    void shutdownTls();
    int tlsSend(const uint8_t* data, int len);
    int tlsRecv(uint8_t* data, int len);

    /// After TLS handshake, pin or verify the leaf certificate SHA-256.
    /// Returns true to allow the connection, false to abort it.
    bool verifyOrPinServerCert();
};
