#pragma once

#include "client_protocol.h"
#include "client_session.h"
#include "stats_reporter.h"
#include "tcp_client.h"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

/// Where and how to connect.
struct SessionSettings {
    std::string serverIp;
    uint16_t controlPort = fvp_session::DEFAULT_CONTROL_PORT;
    uint32_t pin = 0;
    std::string fingerprintStorePath; // TOFU pin file (required)
    int connectTimeoutMs = 5000;
    int handshakeStepTimeoutMs = 10000;
    /// The server answers every HEARTBEAT (500 ms). Nothing from it for
    /// this long means the link is dead, even if TCP has not noticed.
    int livenessTimeoutMs = 3000;
};

/// Something the server (or the link) did, for the app thread.
struct ServerEvent {
    enum class Type {
        Streaming,      // a session started (again); streamConfig() is valid
        Disconnected,   // the session ended; reconnecting
        PinRejected,    // wrong PIN: stays disconnected until start() again
        Haptic,
        SleepEnter,
        SleepExit,
        HeartbeatAck,
        ConfigUpdateAck,
    };
    Type type = Type::Disconnected;
    fvp_client_protocol::HapticEvent haptic{};
    fvp_client_protocol::HeartbeatAck heartbeatAck{};
    uint8_t configKey = 0;
    bool configAccepted = false;
};

/// Runs the connection to the engine on its own thread: connect, pair,
/// stream, and reconnect after a drop, following fvp_session::ClientSession
/// (exponential backoff; a rejected PIN is not retried). That thread is the
/// only one touching the TLS connection; other threads queue messages with
/// send() and read the server's with pollEvent().
///
/// Reconnecting with the same PIN works while the engine holds the dropped
/// session (5 s). After that the engine has a new PIN and the attempt ends
/// in PinRejected.
class StreamSession {
public:
    /// Runs on the session thread after STREAM_CONFIG and before
    /// STREAM_START: start receiving video here, so the first packets —
    /// often the keyframe — are not missed.
    using StreamStartHook =
        std::function<void(const TcpControlClient::StreamConfig&, const std::string& serverIp)>;
    /// Runs on the session thread when a streaming session ends.
    using StreamEndHook = std::function<void()>;

    static constexpr auto kHeartbeatInterval = std::chrono::milliseconds(500);
    /// Queued messages beyond this are dropped oldest first (e.g. FACE_DATA
    /// at 90 Hz while the link stalls).
    static constexpr size_t kMaxOutbox = 128;
    static constexpr size_t kMaxEvents = 256;

    StreamSession() = default;
    ~StreamSession() { stop(); }
    StreamSession(const StreamSession&) = delete;
    StreamSession& operator=(const StreamSession&) = delete;

    void setStreamStartHook(StreamStartHook hook) { m_onStreamStart = std::move(hook); }
    void setStreamEndHook(StreamEndHook hook) { m_onStreamEnd = std::move(hook); }

    /// Start connecting (stops a running session first).
    void start(const SessionSettings& settings);

    /// Leave: send DISCONNECT if streaming (so the engine does not hold the
    /// session for a reconnect), close, and join the thread.
    void stop();

    fvp_session::SessionState state() const {
        return static_cast<fvp_session::SessionState>(m_state.load());
    }
    bool isStreaming() const { return state() == fvp_session::SessionState::Streaming; }

    /// Valid after a Streaming event.
    TcpControlClient::StreamConfig streamConfig() const;

    /// Number of sessions that reached Streaming since start().
    uint32_t streamCount() const { return m_streamCount; }

    /// Queue a message for the server. Dropped unless streaming.
    void send(uint8_t type, const uint8_t* payload, size_t len);

    /// Ask for a keyframe. Requests made while one is queued are merged.
    void requestIdr();

    /// Take the oldest server event, if any.
    bool pollEvent(ServerEvent& out);

    /// Shared with the video receiver and the decoder; reported in every
    /// HEARTBEAT.
    StatsReporter& stats() { return m_stats; }

    /// Test hook: drop the connection without DISCONNECT, as a Wi-Fi
    /// outage would.
    void dropConnectionForTest() { m_dropForTest = true; }

private:
    struct Outgoing {
        uint8_t type;
        std::vector<uint8_t> payload;
    };

    void run();
    /// Serve a streaming session; returns when the link fails or stop().
    void runStreaming();
    void dispatch(uint8_t type, const std::vector<uint8_t>& payload);
    void pushEvent(const ServerEvent& e);
    void setState(fvp_session::SessionState s) { m_state = static_cast<int>(s); }
    /// Sleep up to `ms`, waking early on stop(). Returns false if stopping.
    bool waitFor(uint32_t ms);

    SessionSettings m_settings;
    TcpControlClient m_client;
    StatsReporter m_stats;
    StreamStartHook m_onStreamStart;
    StreamEndHook m_onStreamEnd;

    std::thread m_thread;
    std::atomic<bool> m_stop{false};
    std::atomic<bool> m_dropForTest{false};
    std::atomic<bool> m_idrPending{false};
    std::atomic<int> m_state{static_cast<int>(fvp_session::SessionState::Disconnected)};
    std::atomic<uint32_t> m_streamCount{0};

    mutable std::mutex m_mutex; // everything below
    std::condition_variable m_wake;
    TcpControlClient::StreamConfig m_config;
    std::deque<Outgoing> m_outbox;
    std::deque<ServerEvent> m_events;
};
