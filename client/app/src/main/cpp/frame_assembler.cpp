#include "frame_assembler.h"
#include "client_protocol.h"

#include <algorithm>

namespace proto = fvp_client_protocol;

namespace {

/// Serial-number comparison for the u32 frame index.
bool isNewer(uint32_t a, uint32_t b) {
    return static_cast<int32_t>(a - b) > 0;
}

/// Gaps larger than this (packets or frames) are a restart, not loss.
constexpr uint32_t kMaxCountedGap = 1000;

}  // namespace

void FrameAssembler::reset() {
    // Fresh decoders: a slice context still holding shards of the previous
    // session must not be taken for a new frame with the same index.
    m_bulk = FecFrameDecoder();
    m_sliced = SlicedFecFrameDecoder();
    m_haveFrame = false;
    m_frameFinished = false;
    m_waitingForKeyframe = true;
    m_idrAsked = false;
    m_haveSequence = false;
    m_framesDelivered = 0;
    m_framesLost = 0;
    m_framesSkipped = 0;
}

void FrameAssembler::onPacket(const uint8_t* packet, size_t len, Clock::time_point now, Output& out) {
    proto::FvpHeaderView h;
    if (!proto::parseFvpHeader(packet, len, h)) {
        return; // too short, or inconsistent shard fields
    }
    accountSequence(proto::rtpSequence(packet));

    if (m_haveFrame && h.frameIndex == m_frameIndex) {
        if (m_frameFinished) return; // a late or extra shard of a finished frame
    } else if (!m_haveFrame || isNewer(h.frameIndex, m_frameIndex)) {
        if (m_haveFrame) {
            // The frame in progress will get no more packets, and any frame
            // index skipped over never arrived at all.
            uint64_t lost = m_frameFinished ? 0 : 1;
            lost += std::min<uint32_t>(h.frameIndex - m_frameIndex - 1, kMaxCountedGap);
            if (lost > 0) loseFrames(lost, now, out);
        }
        beginFrame(h.frameIndex, h.flags, h.totalShards, h.dataShards, now);
    } else {
        return; // a late packet of an older frame
    }

    const uint8_t* shard = packet + proto::PACKET_HEADER_LEN;
    const int shardLen = static_cast<int>(len - proto::PACKET_HEADER_LEN);
    if (m_frameSliced) {
        m_sliced.addShard(fvp_flags::sliceIndex(h.flags), h.shardIndex, h.totalShards,
                          h.dataShards, shard, shardLen);
    } else {
        m_bulk.addShard(h.shardIndex, shard, shardLen);
    }
    if (currentComplete()) finishCurrent(now, out);
}

void FrameAssembler::onTick(Clock::time_point now, Output& out) {
    if (m_haveFrame && !m_frameFinished && now - m_frameStart > kFrameTimeout) {
        m_frameFinished = true;
        loseFrames(1, now, out);
    }
}

void FrameAssembler::requireKeyframe(Clock::time_point now, Output& out) {
    m_waitingForKeyframe = true;
    askForIdr(now, out);
}

void FrameAssembler::accountSequence(uint16_t seq) {
    if (m_stats) m_stats->onPacketReceived();
    if (!m_haveSequence) {
        m_haveSequence = true;
        m_expectedSequence = static_cast<uint16_t>(seq + 1);
        return;
    }
    const int16_t diff = static_cast<int16_t>(seq - m_expectedSequence);
    if (diff == 0) {
        m_expectedSequence = static_cast<uint16_t>(seq + 1);
    } else if (diff > 0) {
        if (static_cast<uint32_t>(diff) <= kMaxCountedGap && m_stats) {
            m_stats->onPacketLost(static_cast<uint32_t>(diff));
        }
        m_expectedSequence = static_cast<uint16_t>(seq + 1);
    }
    // diff < 0: reordered or duplicated. It was already counted as lost when
    // the sequence skipped over it; leave the expectation where it is.
}

void FrameAssembler::beginFrame(uint32_t frameIndex, uint16_t flags, uint16_t totalShards,
                                uint16_t dataShards, Clock::time_point now) {
    m_haveFrame = true;
    m_frameIndex = frameIndex;
    m_frameFinished = false;
    m_frameStart = now;
    const uint8_t slices = fvp_flags::sliceCount(flags);
    const bool keyframe = fvp_flags::isKeyframe(flags);
    m_frameSliced = slices > 0;
    if (m_frameSliced) {
        // Each slice's shard counts come with its own packets.
        m_sliced.beginFrame(frameIndex, slices, keyframe);
    } else {
        // parseFvpHeader validated the counts, so this cannot fail.
        m_bulk.beginFrame(frameIndex, totalShards, dataShards, keyframe);
    }
}

bool FrameAssembler::currentComplete() const {
    return m_frameSliced ? m_sliced.isComplete() : m_bulk.isComplete();
}

void FrameAssembler::finishCurrent(Clock::time_point now, Output& out) {
    m_frameFinished = true;

    Frame frame;
    frame.frameIndex = m_frameIndex;
    bool rebuilt = false;
    if (m_frameSliced) {
        if (auto f = m_sliced.tryDecode()) {
            frame.keyframe = f->isKeyframe;
            frame.data = std::move(f->data);
            rebuilt = true;
        }
    } else if (auto f = m_bulk.tryDecode()) {
        frame.keyframe = f->isKeyframe;
        frame.data = std::move(f->data);
        rebuilt = true;
    }
    if (!rebuilt) {
        loseFrames(1, now, out); // enough shards, but Reed-Solomon failed
        return;
    }

    if (m_waitingForKeyframe && !frame.keyframe) {
        // Would decode against a reference the decoder never got.
        m_framesSkipped++;
        askForIdr(now, out);
        return;
    }
    if (frame.keyframe) m_waitingForKeyframe = false;
    m_framesDelivered++;
    out.frames.push_back(std::move(frame));
}

void FrameAssembler::loseFrames(uint64_t count, Clock::time_point now, Output& out) {
    m_framesLost += count;
    m_waitingForKeyframe = true;
    askForIdr(now, out);
}

void FrameAssembler::askForIdr(Clock::time_point now, Output& out) {
    if (m_idrAsked && now - m_lastIdrAsk < kIdrRepeat) return;
    m_idrAsked = true;
    m_lastIdrAsk = now;
    out.requestIdr = true;
}
