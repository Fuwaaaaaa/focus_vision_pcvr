#pragma once

#include "frame_assembler.h"
#include "net_compat.h"
#include "stats_reporter.h"

#include <atomic>
#include <chrono>
#include <cstdint>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>

/// Receives the engine's video packets on their own thread and hands
/// complete frames, in order, to the render thread.
///
/// The render loop is paced by xrWaitFrame, so reading packets there capped
/// the rate at a few thousand packets per second — below what 80 Mbps needs.
/// FEC reconstruction also runs here, off the render thread; popFrame()
/// never waits for it.
///
/// Bind once (start()); each streaming session calls beginSession() with
/// the server's address, which also resets frame assembly. Packets from any
/// other address are dropped.
class VideoReceiver {
public:
    using IdrCallback = std::function<void()>;
    using Clock = std::chrono::steady_clock;

    /// Frames waiting beyond this mean the decoder is not keeping up: they
    /// are dropped and an IDR requested rather than letting latency grow.
    static constexpr size_t kMaxQueuedFrames = 8;

    ~VideoReceiver() { stop(); }

    /// Bind 0.0.0.0:`port` and start the receive thread. `onIdrNeeded` runs
    /// on the receive thread when a keyframe should be requested.
    bool start(uint16_t port, StatsReporter* stats, IdrCallback onIdrNeeded);
    void stop();

    /// Accept packets from `serverIp` as a new session.
    void beginSession(const std::string& serverIp);
    /// Drop packets until the next beginSession().
    void endSession();

    /// Take the oldest complete frame. Render thread.
    bool popFrame(FrameAssembler::Frame& out);

    /// The decoder lost a frame (or was not fed for a while): drop the
    /// queued frames, which reference it, and restart at the next keyframe
    /// (requested now).
    void requireKeyframe();

    /// When the last packet of the current session arrived (or when it
    /// began, before any packet).
    Clock::time_point lastPacketTime() const {
        return Clock::time_point(Clock::duration(m_lastPacket.load()));
    }

    uint64_t framesDelivered() const;
    uint64_t framesLost() const;
    uint64_t framesDropped() const { return m_framesDropped; }

private:
    void run();

    fvp_net::Socket m_socket = fvp_net::kInvalidSocket;
    std::thread m_thread;
    std::atomic<bool> m_stop{false};
    IdrCallback m_onIdrNeeded;
    std::atomic<Clock::rep> m_lastPacket{0};
    std::atomic<uint64_t> m_framesDropped{0};
    /// Bumped by beginSession / endSession, so frames assembled for a
    /// session that ended meanwhile never reach the queue.
    std::atomic<uint64_t> m_generation{0};

    mutable std::mutex m_assemblyMutex; // receive thread vs. session changes
    FrameAssembler m_assembler;
    bool m_sessionActive = false;
    in_addr m_serverAddr{};

    std::mutex m_queueMutex; // receive thread vs. render thread
    std::deque<FrameAssembler::Frame> m_frames;
};
