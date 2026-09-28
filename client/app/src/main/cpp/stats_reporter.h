#pragma once

#include "client_protocol.h"

#include <atomic>
#include <cstdint>

/// Collects HMD-side statistics for the HEARTBEAT the session sends to the
/// PC every 500 ms. Thread-safe: the video receive thread and the render
/// thread record, the session thread takes the snapshot.
class StatsReporter {
public:
    /// Record a received video packet.
    void onPacketReceived() { m_packetsReceived++; }

    /// Record lost video packets (RTP sequence gap).
    void onPacketLost(uint32_t count = 1) { m_packetsLost += count; }

    /// Record a decoded frame with its decode latency.
    void onFrameDecoded(uint32_t decodeLatencyUs) {
        m_framesDecoded++;
        m_totalDecodeLatencyUs += decodeLatencyUs;
    }

    /// Stats for the interval since the previous snapshot, which lasted
    /// `elapsedMs`; resets the counters. The engine's adaptive FEC and
    /// bitrate react to packetsLost / (packetsReceived + packetsLost).
    fvp_client_protocol::HeartbeatStats takeSnapshot(uint32_t elapsedMs) {
        const uint32_t frames = m_framesDecoded.exchange(0);
        const uint64_t totalLatency = m_totalDecodeLatencyUs.exchange(0);
        fvp_client_protocol::HeartbeatStats s;
        s.packetsReceived = m_packetsReceived.exchange(0);
        s.packetsLost = m_packetsLost.exchange(0);
        s.avgDecodeUs = frames > 0 ? static_cast<uint32_t>(totalLatency / frames) : 0;
        const uint64_t fps = elapsedMs > 0 ? (uint64_t)frames * 1000 / elapsedMs : 0;
        s.fps = static_cast<uint16_t>(fps > 0xFFFF ? 0xFFFF : fps);
        return s;
    }

    uint32_t packetsReceived() const { return m_packetsReceived.load(); }
    uint32_t packetsLost() const { return m_packetsLost.load(); }

private:
    std::atomic<uint32_t> m_packetsReceived{0};
    std::atomic<uint32_t> m_packetsLost{0};
    std::atomic<uint32_t> m_framesDecoded{0};
    std::atomic<uint64_t> m_totalDecodeLatencyUs{0};
};
