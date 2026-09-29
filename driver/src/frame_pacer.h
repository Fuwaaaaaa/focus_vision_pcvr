#pragma once

#include <chrono>

/**
 * Paces SteamVR's compositor to the headset's refresh rate. SteamVR times
 * vsync itself (the driver sends no vsync events) and calls PostPresent
 * after every Present; waiting there until the frame's slot ends keeps it
 * from rendering faster than the stream. ALVR paces the same way.
 */
class FramePacer {
public:
    using Clock = std::chrono::steady_clock;

    explicit FramePacer(double refreshHz = 90.0) { setRefreshRate(refreshHz); }

    void setRefreshRate(double refreshHz) {
        const double hz = refreshHz > 0.0 ? refreshHz : 90.0;
        m_period = std::chrono::duration_cast<Clock::duration>(std::chrono::duration<double>(1.0 / hz));
        m_started = false;
    }

    Clock::duration period() const { return m_period; }

    /// Until when the frame that finished at `now` should wait: the end of
    /// its slot on the refresh-rate grid. A frame that is already past its
    /// slot waits for nothing, and the grid restarts from it — the frames
    /// after a stall are not let through back to back to catch up.
    Clock::time_point deadline(Clock::time_point now) {
        const Clock::time_point slotEnd = m_started ? m_next + m_period : now + m_period;
        m_started = true;
        m_next = slotEnd < now ? now : slotEnd;
        return m_next;
    }

private:
    Clock::duration m_period{};
    Clock::time_point m_next{};
    bool m_started = false;
};
