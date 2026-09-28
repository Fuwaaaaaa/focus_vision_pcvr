#include "stream_session.h"
#include "xr_utils.h"

namespace proto = fvp_client_protocol;
using fvp_session::SessionEvent;
using fvp_session::SessionState;
using Clock = std::chrono::steady_clock;

namespace {

/// Longest one wait for a server message may block the session loop, which
/// also sends the queued messages. Bounds how long an IDR request or
/// FACE_DATA waits.
constexpr int kPollMs = 10;

}  // namespace

void StreamSession::start(const SessionSettings& settings) {
    stop();
    m_settings = settings;
    m_stop = false;
    m_dropForTest = false;
    m_idrPending = false;
    m_streamCount = 0;
    {
        std::lock_guard<std::mutex> lock(m_mutex);
        m_outbox.clear();
        m_events.clear();
    }
    m_client.setFingerprintStorePath(settings.fingerprintStorePath);
    m_thread = std::thread([this] { run(); });
}

void StreamSession::stop() {
    {
        std::lock_guard<std::mutex> lock(m_mutex);
        m_stop = true;
    }
    m_wake.notify_all();
    if (m_thread.joinable()) m_thread.join();
    setState(SessionState::Disconnected);
}

TcpControlClient::StreamConfig StreamSession::streamConfig() const {
    std::lock_guard<std::mutex> lock(m_mutex);
    return m_config;
}

void StreamSession::send(uint8_t type, const uint8_t* payload, size_t len) {
    if (!isStreaming()) return;
    std::lock_guard<std::mutex> lock(m_mutex);
    if (m_outbox.size() >= kMaxOutbox) m_outbox.pop_front();
    m_outbox.push_back({type, std::vector<uint8_t>(payload, payload + len)});
}

void StreamSession::requestIdr() {
    if (isStreaming()) m_idrPending = true;
}

bool StreamSession::pollEvent(ServerEvent& out) {
    std::lock_guard<std::mutex> lock(m_mutex);
    if (m_events.empty()) return false;
    out = m_events.front();
    m_events.pop_front();
    return true;
}

void StreamSession::pushEvent(const ServerEvent& e) {
    std::lock_guard<std::mutex> lock(m_mutex);
    if (m_events.size() >= kMaxEvents) m_events.pop_front();
    m_events.push_back(e);
}

bool StreamSession::waitFor(uint32_t ms) {
    std::unique_lock<std::mutex> lock(m_mutex);
    return !m_wake.wait_for(lock, std::chrono::milliseconds(ms), [this] { return m_stop.load(); });
}

void StreamSession::run() {
    fvp_session::ClientSession fsm;
    fsm.on_event(SessionEvent::ConnectRequested);
    setState(fsm.state());

    while (!m_stop) {
        switch (fsm.state()) {
        case SessionState::Connecting: {
            LOGI("Session: connecting to %s:%u", m_settings.serverIp.c_str(), m_settings.controlPort);
            const bool ok = m_client.connect(m_settings.serverIp.c_str(), m_settings.controlPort,
                                             m_settings.connectTimeoutMs);
            fsm.on_event(ok ? SessionEvent::TransportConnected : SessionEvent::TransportFailed);
            break;
        }
        case SessionState::Pairing: {
            const auto result = m_client.handshake(m_settings.pin, m_settings.handshakeStepTimeoutMs);
            if (result == TcpControlClient::HandshakeResult::Ok) {
                fsm.on_event(SessionEvent::HandshakeOk);
            } else if (result == TcpControlClient::HandshakeResult::PinRejected) {
                // Retrying a wrong PIN would only run into the engine's lockout.
                m_client.disconnect();
                fsm.on_event(SessionEvent::PinRejected);
                ServerEvent e;
                e.type = ServerEvent::Type::PinRejected;
                pushEvent(e);
            } else {
                m_client.disconnect();
                fsm.on_event(SessionEvent::HandshakeFailed);
            }
            break;
        }
        case SessionState::Configuring: {
            const TcpControlClient::StreamConfig config = m_client.getStreamConfig();
            {
                std::lock_guard<std::mutex> lock(m_mutex);
                m_config = config;
            }
            if (m_onStreamStart) m_onStreamStart(config, m_settings.serverIp);
            if (m_client.startStream()) {
                fsm.on_event(SessionEvent::StreamConfigured);
                m_streamCount++;
                setState(fsm.state()); // isStreaming() before the app sees the event
                ServerEvent e;
                e.type = ServerEvent::Type::Streaming;
                pushEvent(e);
            } else {
                m_client.disconnect();
                if (m_onStreamEnd) m_onStreamEnd();
                fsm.on_event(SessionEvent::TransportFailed);
            }
            break;
        }
        case SessionState::Streaming: {
            runStreaming();
            if (m_stop) break; // the goodbye below ends the session
            m_client.disconnect();
            {
                std::lock_guard<std::mutex> lock(m_mutex);
                m_outbox.clear();
            }
            m_idrPending = false;
            if (m_onStreamEnd) m_onStreamEnd();
            fsm.on_event(SessionEvent::TransportFailed);
            ServerEvent e;
            e.type = ServerEvent::Type::Disconnected;
            pushEvent(e);
            break;
        }
        case SessionState::Reconnecting: {
            if (fsm.should_warn_flaky()) {
                LOGW("Session: link keeps dropping (%u attempts)", fsm.reconnect_attempts());
            }
            LOGI("Session: reconnecting in %u ms", fsm.backoff_ms());
            if (waitFor(fsm.backoff_ms())) fsm.on_event(SessionEvent::BackoffElapsed);
            break;
        }
        case SessionState::Disconnected: {
            // Only after a rejected PIN: wait for stop() (start() again
            // brings a new PIN).
            std::unique_lock<std::mutex> lock(m_mutex);
            m_wake.wait(lock, [this] { return m_stop.load(); });
            break;
        }
        }
        setState(fsm.state());
    }

    if (fsm.state() == SessionState::Streaming) {
        // Leaving on purpose: DISCONNECT lets the engine end the session now
        // instead of holding it for a reconnect.
        m_client.sendMessage(proto::msg::DISCONNECT, nullptr, 0);
        if (m_onStreamEnd) m_onStreamEnd();
    }
    m_client.disconnect();
}

void StreamSession::runStreaming() {
    using std::chrono::duration_cast;
    using std::chrono::milliseconds;

    const auto liveness = milliseconds(m_settings.livenessTimeoutMs);
    auto lastHeard = Clock::now();
    auto lastHeartbeat = Clock::now() - kHeartbeatInterval; // send one right away
    auto statsSince = Clock::now();
    uint32_t heartbeatSequence = 0;
    std::vector<uint8_t> payload;
    m_stats.takeSnapshot(0); // start this session's counts from zero

    while (!m_stop) {
        if (m_dropForTest.exchange(false)) {
            LOGW("Session: dropping the connection (test hook)");
            return;
        }

        std::deque<Outgoing> outbox;
        {
            std::lock_guard<std::mutex> lock(m_mutex);
            outbox.swap(m_outbox);
        }
        for (const auto& m : outbox) {
            if (!m_client.sendMessage(m.type, m.payload.data(), static_cast<int>(m.payload.size()))) {
                return;
            }
        }
        if (m_idrPending.exchange(false) && !m_client.requestIdr()) return;

        const auto now = Clock::now();
        if (now - lastHeartbeat >= kHeartbeatInterval) {
            const auto elapsedMs = duration_cast<milliseconds>(now - statsSince).count();
            const auto stats = m_stats.takeSnapshot(static_cast<uint32_t>(elapsedMs));
            const auto wallMs = duration_cast<milliseconds>(
                std::chrono::system_clock::now().time_since_epoch()).count();
            const auto heartbeat = proto::buildHeartbeatPayload(
                heartbeatSequence++, static_cast<uint64_t>(wallMs), stats);
            if (!m_client.sendMessage(proto::msg::HEARTBEAT, heartbeat.data(),
                                      static_cast<int>(heartbeat.size()))) {
                return;
            }
            lastHeartbeat = now;
            statsSince = now;
        }

        uint8_t type = 0;
        const auto r = m_client.recvMessage(type, payload, kPollMs);
        if (r == TcpControlClient::RecvResult::Closed) {
            LOGW("Session: control connection closed");
            return;
        }
        if (r == TcpControlClient::RecvResult::Message) {
            lastHeard = Clock::now();
            dispatch(type, payload);
        }
        if (Clock::now() - lastHeard > liveness) {
            LOGW("Session: nothing from the server for %d ms — link lost", m_settings.livenessTimeoutMs);
            return;
        }
    }
}

void StreamSession::dispatch(uint8_t type, const std::vector<uint8_t>& payload) {
    ServerEvent e;
    switch (type) {
    case proto::msg::HEARTBEAT_ACK:
        e.type = ServerEvent::Type::HeartbeatAck;
        proto::parseHeartbeatAck(payload.data(), payload.size(), e.heartbeatAck);
        break;
    case proto::msg::HAPTIC_EVENT:
        if (!proto::parseHapticEvent(payload.data(), payload.size(), e.haptic)) {
            LOGW("Session: HAPTIC_EVENT too short (%zu bytes)", payload.size());
            return;
        }
        e.type = ServerEvent::Type::Haptic;
        break;
    case proto::msg::SLEEP_ENTER:
        e.type = ServerEvent::Type::SleepEnter;
        break;
    case proto::msg::SLEEP_EXIT:
        e.type = ServerEvent::Type::SleepExit;
        break;
    case proto::msg::CONFIG_UPDATE_ACK:
        if (payload.size() < 2) return;
        e.type = ServerEvent::Type::ConfigUpdateAck;
        e.configAccepted = payload[0] == 0x01;
        e.configKey = payload[1];
        break;
    default:
        LOGW("Session: unknown server message 0x%02x (%zu bytes) — skipped", type, payload.size());
        return;
    }
    pushEvent(e);
}
