#pragma once

#include <cstdint>
#include <atomic>

/// Collects HMD-side statistics and reports to PC via TCP control channel.
class StatsReporter {
public:
    /// Record a received video packet.
    void onPacketReceived() { m_packetsReceived++; }

    /// Record a detected lost packet (sequence gap).
    void onPacketLost(uint32_t count = 1) { m_packetsLost += count; }

    /// Record a decoded frame with its latency.
    void onFrameDecoded(uint32_t decodeLatencyUs) {
        m_framesDecoded++;
        m_totalDecodeLatencyUs += decodeLatencyUs;
    }

    /// Generate a stats report payload for TCP transmission.
    /// Format: [packets_received:4B][packets_lost:4B][avg_decode_us:4B][fps:2B]
    /// Resets counters after generating.
    void generateReport(uint8_t* outBuf, int& outLen) {
        uint32_t received = m_packetsReceived.exchange(0);
        uint32_t lost = m_packetsLost.exchange(0);
        uint32_t frames = m_framesDecoded.exchange(0);
        uint64_t totalLatency = m_totalDecodeLatencyUs.exchange(0);

        uint32_t avgDecodeUs = (frames > 0) ? (uint32_t)(totalLatency / frames) : 0;
        uint16_t fps = (uint16_t)frames; // Approximate (if called every second)

        int off = 0;
        memcpy(outBuf + off, &received, 4); off += 4;
        memcpy(outBuf + off, &lost, 4); off += 4;
        memcpy(outBuf + off, &avgDecodeUs, 4); off += 4;
        memcpy(outBuf + off, &fps, 2); off += 2;
        outLen = off; // 14 bytes
    }

    uint32_t packetsReceived() const { return m_packetsReceived.load(); }
    uint32_t packetsLost() const { return m_packetsLost.load(); }

private:
    std::atomic<uint32_t> m_packetsReceived{0};
    std::atomic<uint32_t> m_packetsLost{0};
    std::atomic<uint32_t> m_framesDecoded{0};
    std::atomic<uint64_t> m_totalDecodeLatencyUs{0};
};
