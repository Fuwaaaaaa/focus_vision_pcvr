#pragma once
// Which audio packets went missing, from the RTP sequence numbers of the
// ones that arrive: the player fills each missing 10 ms with Opus's loss
// concealment instead of leaving a gap (a click). The engine's Opus runs
// in its low-delay mode (CELT only), which has no in-band FEC to recover
// from, so concealment is what a loss gets. Pure C++, host-tested
// (client/tests/test_audio_sequence.cpp).

#include <algorithm>
#include <cstdint>

class AudioSequence {
public:
    /// The most packets concealed for one gap (50 ms). A longer gap is a
    /// stall: concealment fades to silence well before then anyway.
    static constexpr int kMaxConcealed = 5;
    /// A packet this far behind is late (or repeated); one further behind
    /// means the sender started over (a new session numbers from 0).
    static constexpr int kReorderWindow = 50;

    /// Returns how many packets are missing just before `sequence` (0 if
    /// none, at most kMaxConcealed), or -1 for a late or repeated packet:
    /// its moment has passed, so the caller drops it.
    int onPacket(uint16_t sequence) {
        if (!m_started) {
            m_started = true;
            m_last = sequence;
            return 0;
        }
        const int delta = static_cast<int16_t>(static_cast<uint16_t>(sequence - m_last));
        if (delta <= 0 && delta >= -kReorderWindow) return -1;
        m_last = sequence;
        if (delta < 0) return 0;  // the sender started over
        return std::min(delta - 1, kMaxConcealed);
    }

    /// Forget the numbering (a new session).
    void reset() { m_started = false; }

private:
    bool m_started = false;
    uint16_t m_last = 0;
};
