#pragma once
// The render poses of the frames sent to the decoder (protocol v6), looked
// up again when the decoder hands a frame back: MediaCodec keeps the
// presentation timestamp, from which the frame index follows. Pure C++,
// host-tested (client/tests/test_render_pose_log.cpp).

#include <array>
#include <cstddef>
#include <cstdint>

#include "client_protocol.h"

class RenderPoseLog {
public:
    /// The frames the decoder can hold at once are far fewer.
    static constexpr size_t kSize = 32;

    void record(uint32_t frameIndex, const fvp_client_protocol::FramePose& pose) {
        Entry& e = m_entries[frameIndex % kSize];
        e.frameIndex = frameIndex;
        e.pose = pose;
        e.used = true;
    }

    /// The orientation frame `frameIndex` was rendered at, if it was among
    /// the last kSize recorded and known.
    bool find(uint32_t frameIndex, float orientation[4]) const {
        const Entry& e = m_entries[frameIndex % kSize];
        if (!e.used || e.frameIndex != frameIndex || !e.pose.known) return false;
        for (int i = 0; i < 4; i++) orientation[i] = e.pose.orientation[i];
        return true;
    }

    /// The frame index of presentation timestamp `ptsUs`, as
    /// frameIndex × frameDurationUs was submitted.
    static uint32_t frameIndexOf(int64_t ptsUs, uint32_t frameDurationUs) {
        if (ptsUs < 0 || frameDurationUs == 0) return UINT32_MAX;
        return static_cast<uint32_t>((ptsUs + frameDurationUs / 2) / frameDurationUs);
    }

    void clear() { m_entries = {}; }

private:
    struct Entry {
        uint32_t frameIndex = 0;
        fvp_client_protocol::FramePose pose;
        bool used = false;
    };
    std::array<Entry, kSize> m_entries{};
};
