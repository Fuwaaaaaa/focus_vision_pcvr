// Host-buildable unit tests for the pure client session state machine. This
// encodes the connect -> pair -> configure -> stream -> reconnect ORCHESTRATION
// POLICY (state transitions + reconnect backoff) with no sockets / OpenXR /
// MediaCodec, so the policy is verifiable without the device. The orchestrator
// (which owns the real I/O) feeds events and acts on the resulting state.
#include <gtest/gtest.h>

#include "client_session.h"

using namespace fvp_session;

TEST(ClientSession, StartsDisconnected) {
    ClientSession s;
    EXPECT_EQ(s.state(), SessionState::Disconnected);
    EXPECT_EQ(s.reconnect_attempts(), 0u);
}

TEST(ClientSession, HappyPathToStreaming) {
    ClientSession s;
    EXPECT_EQ(s.on_event(SessionEvent::ConnectRequested), SessionState::Connecting);
    EXPECT_EQ(s.on_event(SessionEvent::TransportConnected), SessionState::Pairing);
    EXPECT_EQ(s.on_event(SessionEvent::HandshakeOk), SessionState::Configuring);
    EXPECT_EQ(s.on_event(SessionEvent::StreamConfigured), SessionState::Streaming);
}

TEST(ClientSession, PinRejectedDoesNotAutoReconnect) {
    // Security: a wrong PIN will not fix itself — require an explicit re-pair
    // rather than hammering the server (which would trip its lockout).
    ClientSession s;
    s.on_event(SessionEvent::ConnectRequested);
    s.on_event(SessionEvent::TransportConnected);
    EXPECT_EQ(s.on_event(SessionEvent::PinRejected), SessionState::Disconnected);
    EXPECT_EQ(s.reconnect_attempts(), 0u);
}

TEST(ClientSession, TransportFailureReconnectsAndCountsAttempts) {
    ClientSession s;
    s.on_event(SessionEvent::ConnectRequested);
    EXPECT_EQ(s.on_event(SessionEvent::TransportFailed), SessionState::Reconnecting);
    EXPECT_EQ(s.reconnect_attempts(), 1u);
    EXPECT_EQ(s.on_event(SessionEvent::BackoffElapsed), SessionState::Connecting);
    EXPECT_EQ(s.on_event(SessionEvent::TransportFailed), SessionState::Reconnecting);
    EXPECT_EQ(s.reconnect_attempts(), 2u);
}

TEST(ClientSession, MidStreamTimeoutReconnects) {
    ClientSession s;
    s.on_event(SessionEvent::ConnectRequested);
    s.on_event(SessionEvent::TransportConnected);
    s.on_event(SessionEvent::HandshakeOk);
    s.on_event(SessionEvent::StreamConfigured);
    ASSERT_EQ(s.state(), SessionState::Streaming);
    EXPECT_EQ(s.on_event(SessionEvent::PacketTimeout), SessionState::Reconnecting);
    EXPECT_EQ(s.reconnect_attempts(), 1u);
}

TEST(ClientSession, SuccessfulStreamResetsBackoff) {
    ClientSession s;
    s.on_event(SessionEvent::ConnectRequested);
    s.on_event(SessionEvent::TransportFailed); // attempts = 1
    s.on_event(SessionEvent::BackoffElapsed);
    s.on_event(SessionEvent::TransportConnected);
    s.on_event(SessionEvent::HandshakeOk);
    EXPECT_EQ(s.on_event(SessionEvent::StreamConfigured), SessionState::Streaming);
    EXPECT_EQ(s.reconnect_attempts(), 0u) << "a good stream clears the backoff counter";
}

TEST(ClientSession, DisconnectRequestedFromAnyStateGoesDisconnected) {
    ClientSession s;
    s.on_event(SessionEvent::ConnectRequested);
    s.on_event(SessionEvent::TransportConnected);
    EXPECT_EQ(s.on_event(SessionEvent::DisconnectRequested), SessionState::Disconnected);
    EXPECT_EQ(s.reconnect_attempts(), 0u);
}

TEST(ClientSession, BackoffIsExponentialAndCapped) {
    // Mirrors server reconnect.rs: base 1s, x2^(attempt-1), capped at 16s.
    EXPECT_EQ(reconnect_backoff_ms(0), 0u);
    EXPECT_EQ(reconnect_backoff_ms(1), 1000u);
    EXPECT_EQ(reconnect_backoff_ms(2), 2000u);
    EXPECT_EQ(reconnect_backoff_ms(3), 4000u);
    EXPECT_EQ(reconnect_backoff_ms(4), 8000u);
    EXPECT_EQ(reconnect_backoff_ms(5), 16000u);
    EXPECT_EQ(reconnect_backoff_ms(6), 16000u);  // capped
    EXPECT_EQ(reconnect_backoff_ms(50), 16000u); // capped
}

TEST(ClientSession, FlakyLinkWarnsAfterSoftCapButKeepsTrying) {
    ClientSession s;
    s.on_event(SessionEvent::ConnectRequested);
    for (uint32_t i = 0; i < MAX_RECONNECT_ATTEMPTS + 1; i++) {
        s.on_event(SessionEvent::TransportFailed);
        s.on_event(SessionEvent::BackoffElapsed);
    }
    EXPECT_TRUE(s.should_warn_flaky());
    // Never permanently gives up — still cycling, not stuck in Disconnected.
    EXPECT_NE(s.state(), SessionState::Disconnected);
}
