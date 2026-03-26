#pragma once

#include "tcp_client.h"
#include "stats_reporter.h"
#include <cstdint>
#include <chrono>

/// HMD-side heartbeat: sends periodic heartbeats via TCP, includes stats report.
class HeartbeatClient {
public:
    static constexpr int INTERVAL_MS = 500;

    void init(TcpControlClient* tcpClient, StatsReporter* stats) {
        m_tcp = tcpClient;
        m_stats = stats;
        m_lastSend = std::chrono::steady_clock::now();
        m_sequence = 0;
    }

    /// Call every frame. Sends heartbeat if interval has elapsed.
    void tick() {
        if (!m_tcp || !m_tcp->isConnected()) return;

        auto now = std::chrono::steady_clock::now();
        auto elapsed = std::chrono::duration_cast<std::chrono::milliseconds>(
            now - m_lastSend).count();

        if (elapsed < INTERVAL_MS) return;

        // Build heartbeat payload: [sequence:4B][timestamp_ms:8B][stats:14B]
        uint8_t payload[26];
        int off = 0;

        memcpy(payload + off, &m_sequence, 4); off += 4;
        uint64_t ts = std::chrono::duration_cast<std::chrono::milliseconds>(
            now.time_since_epoch()).count();
        memcpy(payload + off, &ts, 8); off += 8;

        // Append stats
        int statsLen = 0;
        m_stats->generateReport(payload + off, statsLen);
        off += statsLen;

        m_tcp->sendMessage(0x10, payload, off); // MSG_HEARTBEAT
        m_sequence++;
        m_lastSend = now;
    }

private:
    TcpControlClient* m_tcp = nullptr;
    StatsReporter* m_stats = nullptr;
    std::chrono::steady_clock::time_point m_lastSend;
    uint32_t m_sequence = 0;
};
