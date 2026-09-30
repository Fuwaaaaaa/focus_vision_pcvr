#pragma once

#include "fec_decoder.h"
#include "stats_reporter.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <vector>

/// Turns received video packets (RTP + FVP header + FEC shard) into complete
/// frames, in order, for the video decoder. No sockets and no threads, so the
/// host tests drive it directly.
///
/// - A frame is handed over as soon as it can be rebuilt, bulk or sliced,
///   not when the next frame's first packet arrives. Frames therefore leave
///   in the order they were sent, even when bulk and sliced frames alternate
///   (small P-frames go bulk, IDRs sliced).
/// - A frame that cannot be rebuilt — too few shards, a frame index that
///   never arrived, or packets that stopped for kFrameTimeout — is lost.
///   Frames after it would decode against a missing reference, so the
///   assembler asks for an IDR and drops non-key frames until one arrives.
///   The same holds at the start of a session.
/// - A late packet of an already finished frame is ignored instead of
///   restarting that frame.
/// - RTP sequence gaps are counted as lost packets for the HEARTBEAT stats.
class FrameAssembler {
public:
    struct Frame {
        uint32_t frameIndex = 0;
        bool keyframe = false;
        std::vector<uint8_t> data; // Annex B; bulk frames keep FEC zero padding
    };

    /// What one call produced.
    struct Output {
        std::vector<Frame> frames; // in send order
        bool requestIdr = false;   // ask the server for a keyframe
    };

    using Clock = std::chrono::steady_clock;

    /// Longest an incomplete frame may go without a packet before it counts
    /// as lost. From the frame's last packet read, not its first: a receive
    /// thread held up mid-frame finds the rest of it queued in the socket.
    static constexpr auto kFrameTimeout = std::chrono::milliseconds(100);
    /// While waiting for a keyframe, repeat the IDR request this often (the
    /// engine rate-limits them to one per 500 ms).
    static constexpr auto kIdrRepeat = std::chrono::milliseconds(500);

    explicit FrameAssembler(StatsReporter* stats = nullptr) : m_stats(stats) {}

    /// Forget everything; the next frame starts a new session.
    void reset();

    /// Feed one received UDP payload.
    void onPacket(const uint8_t* packet, size_t len, Clock::time_point now, Output& out);

    /// Check the frame in progress for a timeout. Call regularly, packet or
    /// not (VideoReceiver does after every receive).
    void onTick(Clock::time_point now, Output& out);

    /// Drop non-key frames until the next keyframe (the frame queue
    /// overflowed, so the decoder is missing frames), and ask for one.
    void requireKeyframe(Clock::time_point now, Output& out);

    uint64_t framesDelivered() const { return m_framesDelivered; }
    uint64_t framesLost() const { return m_framesLost; }
    uint64_t framesSkipped() const { return m_framesSkipped; }
    bool waitingForKeyframe() const { return m_waitingForKeyframe; }

private:
    void accountSequence(uint16_t seq);
    void beginFrame(uint32_t frameIndex, uint16_t flags, uint16_t totalShards,
                    uint16_t dataShards, Clock::time_point now);
    bool currentComplete() const;
    void finishCurrent(Clock::time_point now, Output& out);
    void loseFrames(uint64_t count, Clock::time_point now, Output& out);
    void askForIdr(Clock::time_point now, Output& out);

    StatsReporter* m_stats;

    FecFrameDecoder m_bulk;
    SlicedFecFrameDecoder m_sliced;

    bool m_haveFrame = false;     // m_frameIndex is set
    uint32_t m_frameIndex = 0;    // the newest frame seen
    bool m_frameSliced = false;
    bool m_frameFinished = false; // delivered or lost
    Clock::time_point m_lastFramePacket; // the frame in progress's newest

    bool m_waitingForKeyframe = true;
    bool m_idrAsked = false;
    Clock::time_point m_lastIdrAsk;

    bool m_haveSequence = false;
    uint16_t m_expectedSequence = 0;

    uint64_t m_framesDelivered = 0;
    uint64_t m_framesLost = 0;
    uint64_t m_framesSkipped = 0;
};
