#pragma once

#include <atomic>
#include <cstdint>
#include <string>
#include <vector>

#include <mbedtls/ctr_drbg.h>
#include <mbedtls/entropy.h>
#include <mbedtls/net_sockets.h>
#include <mbedtls/ssl.h>

/// TLS control channel to the engine: connect, pair with the PIN, receive
/// STREAM_CONFIG, then exchange framed messages
/// ([length u32 LE][type u8][payload]).
///
/// Security model: TLS 1.3 (the engine's rustls accepts nothing older) with
/// TOFU certificate pinning. The caller MUST call setFingerprintStorePath()
/// before connect(). The first server seen is pinned only once pairing with
/// it succeeds, so reaching a wrong address does not pin that host.
///
/// Not thread-safe: one thread (StreamSession's) owns an instance. MbedTLS
/// does not allow reading and writing one connection from two threads.
class TcpControlClient {
public:
    struct StreamConfig {
        uint32_t width = 0;          // native render (target) resolution
        uint32_t height = 0;
        uint32_t bitrateMbps = 0;
        uint32_t framerate = 0;
        uint8_t codec = 1; // 0=H264, 1=H265
        uint32_t encodedWidth = 0;   // actually-decoded resolution (== native unless downscaled)
        uint32_t encodedHeight = 0;
    };

    enum class HandshakeResult { Ok, PinRejected, Failed };
    enum class RecvResult { Message, Timeout, Closed };

    TcpControlClient();
    ~TcpControlClient();
    TcpControlClient(const TcpControlClient&) = delete;
    TcpControlClient& operator=(const TcpControlClient&) = delete;

    /// File that persists the pinned server certificate SHA-256.
    /// Recommended: <ANativeActivity::internalDataPath>/server_fingerprint.hex
    void setFingerprintStorePath(std::string path) { m_fingerprintStorePath = std::move(path); }

    /// Forget the pinned fingerprint (the user re-pairs with another PC).
    void clearPinnedFingerprint();

    /// TCP connect (giving up after `timeoutMs`), TLS handshake, and the
    /// check against the pinned fingerprint.
    bool connect(const char* serverAddress, int port, int timeoutMs = 5000);

    /// Close the connection. Safe to call at any time, repeatedly.
    void disconnect();

    /// HELLO → PIN → STREAM_CONFIG, each step waiting at most
    /// `stepTimeoutMs`. On Ok, getStreamConfig() is valid and the server
    /// waits for startStream().
    HandshakeResult handshake(uint32_t pin, int stepTimeoutMs = 10000);

    /// Send STREAM_START: the server starts sending video after this.
    bool startStream();

    const StreamConfig& getStreamConfig() const { return m_config; }
    bool isConnected() const { return m_connected; }

    /// Request an IDR keyframe from the server.
    bool requestIdr();

    /// Send one framed message.
    bool sendMessage(uint8_t type, const uint8_t* payload, int payloadLen);

    /// Wait up to `timeoutMs` for one complete message.
    RecvResult recvMessage(uint8_t& outType, std::vector<uint8_t>& outPayload, int timeoutMs);

private:
    std::atomic<bool> m_connected{false};
    bool m_tlsEstablished = false;
    StreamConfig m_config;

    mbedtls_ssl_context m_ssl;
    mbedtls_ssl_config m_sslConf;
    mbedtls_entropy_context m_entropy;
    mbedtls_ctr_drbg_context m_ctrDrbg;
    mbedtls_net_context m_netCtx;

    // Decrypted bytes not yet framed into a message.
    std::vector<uint8_t> m_rxBuf;

    // TOFU pinning state.
    std::string m_fingerprintStorePath; // empty == not configured (connect refused)
    std::string m_pinnedFingerprint;    // hex sha256, loaded from disk on first verify
    std::string m_peerFingerprint;      // this connection's server
    bool m_peerNeedsPinning = false;    // no pin yet: pin m_peerFingerprint once paired

    void initContexts();
    void freeContexts();
    bool tlsHandshake(int timeoutMs);
    bool checkServerCert();
    bool pinPeerFingerprint();
    bool writeAll(const uint8_t* data, size_t len);
    /// 1 = a message was taken from m_rxBuf, 0 = incomplete, -1 = bad frame.
    int takeMessage(uint8_t& outType, std::vector<uint8_t>& outPayload);
    bool expectMessage(uint8_t type, std::vector<uint8_t>& payload, int timeoutMs);
};
