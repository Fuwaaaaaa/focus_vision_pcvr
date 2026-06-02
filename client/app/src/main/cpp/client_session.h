#pragma once
// Pure client streaming-session state machine.
//
// Encodes the connect -> pair -> configure -> stream -> reconnect ORCHESTRATION
// POLICY only — no sockets, OpenXR, or MediaCodec — so it is host-buildable and
// unit-tested (see client/tests/test_client_session.cpp). The orchestrator that
// owns the real I/O drives this by feeding events and reacting to the resulting
// state (e.g. on entering Connecting, call TcpControlClient::connect(); on
// Reconnecting, sleep backoff_ms() then emit BackoffElapsed).
//
// Reconnect policy mirrors the engine side (rust/.../control/reconnect.rs):
// exponential backoff (base 1 s, x2 per attempt, capped at 16 s) and a soft
// attempt cap that only drives a "flaky link" warning — the client never
// permanently gives up on a dropped Wi-Fi link.
#include <cstdint>

namespace fvp_session {

enum class SessionState {
    Disconnected,  // idle; nothing in flight
    Connecting,    // TCP+TLS connect in progress
    Pairing,       // HELLO + PIN handshake
    Configuring,   // awaiting STREAM_CONFIG
    Streaming,     // receiving video
    Reconnecting,  // backing off before another connect attempt
};

enum class SessionEvent {
    ConnectRequested,     // app/user asked to connect
    TransportConnected,   // TCP+TLS established
    TransportFailed,      // connect attempt failed
    HandshakeOk,          // PIN accepted
    PinRejected,          // wrong PIN — needs explicit re-pair
    HandshakeFailed,      // other handshake error
    StreamConfigured,     // STREAM_CONFIG received, ready to stream
    PacketTimeout,        // no video packets within the disconnect window
    BackoffElapsed,       // reconnect timer fired
    DisconnectRequested,  // app/user asked to disconnect
};

/// Soft cap on reconnect attempts. Past this we only warn about a flaky link —
/// we never stop trying (mirrors the engine's MAX_RECONNECT_ATTEMPTS).
inline constexpr uint32_t MAX_RECONNECT_ATTEMPTS = 10;

/// Backoff before the next reconnect attempt, in milliseconds: base 1 s shifted
/// left by (attempt-1), capped at 16 s. attempt 0 (not reconnecting) is 0.
inline uint32_t reconnect_backoff_ms(uint32_t attempt) {
    if (attempt == 0) {
        return 0;
    }
    uint32_t shift = attempt - 1;
    if (shift > 4) {
        shift = 4;  // cap at 2^4 = 16x base
    }
    return 1000u << shift;
}

class ClientSession {
public:
    SessionState state() const { return state_; }
    uint32_t reconnect_attempts() const { return attempts_; }

    /// True once the link has dropped more than the soft cap — drives a UI
    /// warning. Does NOT stop reconnection.
    bool should_warn_flaky() const { return attempts_ > MAX_RECONNECT_ATTEMPTS; }

    /// Backoff before the next reconnect attempt (0 unless Reconnecting).
    uint32_t backoff_ms() const {
        return state_ == SessionState::Reconnecting ? reconnect_backoff_ms(attempts_) : 0;
    }

    /// Apply an event and return the new state.
    SessionState on_event(SessionEvent e) {
        // A disconnect request always wins and clears reconnect state.
        if (e == SessionEvent::DisconnectRequested) {
            attempts_ = 0;
            state_ = SessionState::Disconnected;
            return state_;
        }
        switch (state_) {
            case SessionState::Disconnected:
                if (e == SessionEvent::ConnectRequested) state_ = SessionState::Connecting;
                break;
            case SessionState::Connecting:
                if (e == SessionEvent::TransportConnected) state_ = SessionState::Pairing;
                else if (e == SessionEvent::TransportFailed) enter_reconnect();
                break;
            case SessionState::Pairing:
                if (e == SessionEvent::HandshakeOk) state_ = SessionState::Configuring;
                else if (e == SessionEvent::PinRejected) {
                    // Do NOT auto-reconnect a rejected PIN — require re-pair.
                    attempts_ = 0;
                    state_ = SessionState::Disconnected;
                } else if (e == SessionEvent::HandshakeFailed) enter_reconnect();
                break;
            case SessionState::Configuring:
                if (e == SessionEvent::StreamConfigured) {
                    attempts_ = 0;  // a good stream clears the backoff counter
                    state_ = SessionState::Streaming;
                } else if (e == SessionEvent::TransportFailed
                        || e == SessionEvent::PacketTimeout) {
                    enter_reconnect();
                }
                break;
            case SessionState::Streaming:
                if (e == SessionEvent::PacketTimeout || e == SessionEvent::TransportFailed) {
                    enter_reconnect();
                }
                break;
            case SessionState::Reconnecting:
                if (e == SessionEvent::BackoffElapsed) state_ = SessionState::Connecting;
                break;
        }
        return state_;
    }

private:
    void enter_reconnect() {
        attempts_++;
        state_ = SessionState::Reconnecting;
    }

    SessionState state_ = SessionState::Disconnected;
    uint32_t attempts_ = 0;
};

}  // namespace fvp_session
