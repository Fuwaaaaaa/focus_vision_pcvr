// FrameAssembler: received video packets → complete frames, in order.
//
// Packets are built here in the engine's wire format (12-byte RTP header,
// 12-byte FVP header, one shard). Frames are sent without losing data
// shards, so parity content never matters; Reed-Solomon recovery itself is
// covered by test_fec_decoder.cpp.

#include <gtest/gtest.h>

#include "frame_assembler.h"

#include <chrono>
#include <cstdint>
#include <vector>

namespace {

using Packet = std::vector<uint8_t>;
using Clock = FrameAssembler::Clock;
using namespace std::chrono_literals;

constexpr size_t kShard = 8; // small shards keep the frames readable

/// Builds packets like the engine's pipeline.rs, numbering RTP sequence
/// numbers consecutively across frames.
struct Sender {
    uint16_t seq = 100;

    Packet packet(uint32_t frame, uint16_t shard, uint16_t total, uint16_t data,
                  uint16_t flags, const uint8_t* payload) {
        Packet p(24 + kShard, 0);
        p[0] = 0x80;
        p[1] = 97;
        p[2] = static_cast<uint8_t>(seq >> 8);
        p[3] = static_cast<uint8_t>(seq);
        seq++;
        auto u16 = [&](size_t at, uint16_t v) {
            p[at] = static_cast<uint8_t>(v);
            p[at + 1] = static_cast<uint8_t>(v >> 8);
        };
        for (int i = 0; i < 4; i++) p[12 + i] = static_cast<uint8_t>(frame >> (8 * i));
        u16(16, shard);
        u16(18, total);
        u16(20, flags);
        u16(22, data);
        std::copy(payload, payload + kShard, p.begin() + 24);
        return p;
    }

    /// One bulk code word: data shards (zero-padded) then `parity` parity shards.
    std::vector<Packet> bulk(uint32_t frame, bool key, const std::vector<uint8_t>& bytes,
                             uint16_t parity = 1) {
        const uint16_t data = static_cast<uint16_t>((bytes.size() + kShard - 1) / kShard);
        std::vector<uint8_t> padded(bytes);
        padded.resize(static_cast<size_t>(data) * kShard, 0);
        const uint8_t zeros[kShard] = {};
        std::vector<Packet> out;
        const uint16_t flags = key ? 1 : 0;
        for (uint16_t i = 0; i < data; i++) {
            out.push_back(packet(frame, i, data + parity, data, flags, padded.data() + i * kShard));
        }
        for (uint16_t i = 0; i < parity; i++) {
            out.push_back(packet(frame, data + i, data + parity, data, flags, zeros));
        }
        return out;
    }

    /// A sliced frame: each slice is its own code word whose data starts
    /// with a u32 LE length prefix (pipeline.rs encode_frame_sliced).
    std::vector<Packet> sliced(uint32_t frame, bool key, const std::vector<std::vector<uint8_t>>& slices) {
        std::vector<Packet> out;
        const uint8_t count = static_cast<uint8_t>(slices.size());
        for (uint8_t s = 0; s < count; s++) {
            std::vector<uint8_t> prefixed(4);
            for (int i = 0; i < 4; i++) prefixed[i] = static_cast<uint8_t>(slices[s].size() >> (8 * i));
            prefixed.insert(prefixed.end(), slices[s].begin(), slices[s].end());
            const uint16_t data = static_cast<uint16_t>((prefixed.size() + kShard - 1) / kShard);
            prefixed.resize(static_cast<size_t>(data) * kShard, 0);
            const uint16_t flags = static_cast<uint16_t>((key ? 1 : 0) | (s << 1) | (count << 5));
            for (uint16_t i = 0; i < data; i++) {
                out.push_back(packet(frame, i, data, data, flags, prefixed.data() + i * kShard));
            }
        }
        return out;
    }
};

std::vector<uint8_t> bytes(size_t n, uint8_t seed) {
    std::vector<uint8_t> v(n);
    for (size_t i = 0; i < n; i++) v[i] = static_cast<uint8_t>(seed + i);
    return v;
}

struct Harness {
    StatsReporter stats;
    FrameAssembler assembler{&stats};
    Clock::time_point now = Clock::now();
    std::vector<FrameAssembler::Frame> frames;
    int idrRequests = 0;

    void feed(const Packet& p) {
        FrameAssembler::Output out;
        assembler.onPacket(p.data(), p.size(), now, out);
        collect(out);
    }
    void feed(const std::vector<Packet>& ps) {
        for (const auto& p : ps) feed(p);
    }
    void tick() {
        FrameAssembler::Output out;
        assembler.onTick(now, out);
        collect(out);
    }
    void collect(FrameAssembler::Output& out) {
        for (auto& f : out.frames) frames.push_back(std::move(f));
        if (out.requestIdr) idrRequests++;
    }
};

}  // namespace

TEST(FrameAssembler, BulkFrameIsDeliveredOnceItsDataShardsArrive) {
    Harness h;
    Sender tx;
    const auto payload = bytes(20, 1); // 3 data shards + 1 parity
    const auto packets = tx.bulk(7, true, payload);
    ASSERT_EQ(packets.size(), 4u);

    h.feed(packets[0]);
    h.feed(packets[1]);
    EXPECT_TRUE(h.frames.empty());
    h.feed(packets[2]); // last data shard: complete without waiting for parity
    ASSERT_EQ(h.frames.size(), 1u);
    EXPECT_EQ(h.frames[0].frameIndex, 7u);
    EXPECT_TRUE(h.frames[0].keyframe);
    ASSERT_GE(h.frames[0].data.size(), payload.size());
    EXPECT_TRUE(std::equal(payload.begin(), payload.end(), h.frames[0].data.begin()));

    h.feed(packets[3]); // parity of a delivered frame: nothing more
    EXPECT_EQ(h.frames.size(), 1u);
    EXPECT_EQ(h.idrRequests, 0);
}

TEST(FrameAssembler, SlicedAndBulkFramesLeaveInSendOrder) {
    // REGRESSION: the render loop handed a finished frame over only when the
    // next frame of the same kind arrived, flushing bulk before sliced, so a
    // bulk P-frame could reach the decoder before the sliced IDR it follows.
    Harness h;
    Sender tx;
    const auto idr0 = bytes(13, 10);
    const auto idr1 = bytes(11, 40);
    h.feed(tx.sliced(1, true, {idr0, idr1}));
    ASSERT_EQ(h.frames.size(), 1u) << "sliced frame delivered when its last slice completes";
    h.feed(tx.bulk(2, false, bytes(9, 70)));
    h.feed(tx.sliced(3, false, {bytes(5, 90), bytes(6, 99)}));
    h.feed(tx.bulk(4, false, bytes(3, 120)));

    ASSERT_EQ(h.frames.size(), 4u);
    for (uint32_t i = 0; i < 4; i++) EXPECT_EQ(h.frames[i].frameIndex, i + 1);

    // The slices are joined with their length prefixes removed.
    std::vector<uint8_t> joined(idr0);
    joined.insert(joined.end(), idr1.begin(), idr1.end());
    EXPECT_EQ(h.frames[0].data, joined);
    EXPECT_EQ(h.idrRequests, 0);
}

TEST(FrameAssembler, NonKeyFramesBeforeTheFirstKeyframeAreSkipped) {
    Harness h;
    Sender tx;
    h.feed(tx.bulk(1, false, bytes(9, 1))); // a session can start mid-GOP
    EXPECT_TRUE(h.frames.empty());
    EXPECT_EQ(h.idrRequests, 1);
    EXPECT_EQ(h.assembler.framesSkipped(), 1u);

    h.feed(tx.bulk(2, true, bytes(9, 2)));
    h.feed(tx.bulk(3, false, bytes(9, 3)));
    ASSERT_EQ(h.frames.size(), 2u);
    EXPECT_EQ(h.frames[0].frameIndex, 2u);
    EXPECT_EQ(h.frames[1].frameIndex, 3u);
}

TEST(FrameAssembler, IncompleteFrameIsLostAndDecodingWaitsForAKeyframe) {
    Harness h;
    Sender tx;
    h.feed(tx.bulk(1, true, bytes(9, 1)));
    ASSERT_EQ(h.frames.size(), 1u);

    auto broken = tx.bulk(2, false, bytes(20, 2), /*parity=*/1);
    broken.erase(broken.begin() + 1); // a data shard lost
    broken.pop_back();                // and the parity that could rebuild it
    h.feed(broken);
    EXPECT_EQ(h.frames.size(), 1u);
    EXPECT_EQ(h.idrRequests, 0) << "not lost until the next frame starts";

    h.feed(tx.bulk(3, false, bytes(9, 3))); // would decode against frame 2
    EXPECT_EQ(h.assembler.framesLost(), 1u);
    EXPECT_EQ(h.idrRequests, 1);
    EXPECT_EQ(h.frames.size(), 1u);

    h.feed(tx.bulk(4, true, bytes(9, 4)));
    h.feed(tx.bulk(5, false, bytes(9, 5)));
    ASSERT_EQ(h.frames.size(), 3u);
    EXPECT_EQ(h.frames[1].frameIndex, 4u);
    EXPECT_EQ(h.frames[2].frameIndex, 5u);
}

TEST(FrameAssembler, FrameIndexThatNeverArrivedIsLost) {
    Harness h;
    Sender tx;
    h.feed(tx.bulk(10, true, bytes(9, 1)));
    h.feed(tx.bulk(13, false, bytes(9, 2))); // 11 and 12 never came
    EXPECT_EQ(h.assembler.framesLost(), 2u);
    EXPECT_EQ(h.idrRequests, 1);
    ASSERT_EQ(h.frames.size(), 1u);
}

TEST(FrameAssembler, LatePacketOfAFinishedFrameIsIgnored) {
    // REGRESSION: one late packet of frame N-1 restarted frame N-1 and threw
    // away frame N in progress.
    Harness h;
    Sender tx;
    const auto first = tx.bulk(1, true, bytes(9, 1));
    h.feed(first);
    const auto second = tx.bulk(2, false, bytes(20, 2));
    h.feed(second[0]);
    h.feed(first[0]); // late duplicate of frame 1
    h.feed(second[1]);
    h.feed(second[2]);
    ASSERT_EQ(h.frames.size(), 2u);
    EXPECT_EQ(h.frames[1].frameIndex, 2u);
    EXPECT_EQ(h.assembler.framesLost(), 0u);
    EXPECT_EQ(h.idrRequests, 0);
}

TEST(FrameAssembler, StalledFrameTimesOut) {
    Harness h;
    Sender tx;
    h.feed(tx.bulk(1, true, bytes(9, 1)));
    const auto partial = tx.bulk(2, false, bytes(20, 2));
    h.feed(partial[0]);

    h.now += 50ms;
    h.tick();
    EXPECT_EQ(h.assembler.framesLost(), 0u);
    h.now += FrameAssembler::kFrameTimeout;
    h.tick();
    EXPECT_EQ(h.assembler.framesLost(), 1u);
    EXPECT_EQ(h.idrRequests, 1);
    EXPECT_TRUE(h.assembler.waitingForKeyframe());
}

TEST(FrameAssembler, AReceiverPausedMidFrameKeepsTheFrame) {
    // REGRESSION: the timeout ran from the frame's first packet, so a
    // receive thread held up for over 100 ms mid-frame (a busy CI runner; a
    // scheduling hiccup on the headset) threw away a frame whose remaining
    // packets were already waiting in the socket, and then skipped frames
    // until the next keyframe.
    Harness h;
    Sender tx;
    h.feed(tx.bulk(1, true, bytes(9, 1)));
    const auto frame = tx.bulk(2, false, bytes(20, 2));
    h.feed(frame[0]);
    h.tick();

    h.now += 150ms; // the thread was paused; the rest was queued meanwhile
    h.feed(frame[1]);
    h.tick();       // what VideoReceiver does after each packet
    h.feed(frame[2]);
    h.tick();
    EXPECT_EQ(h.assembler.framesLost(), 0u);
    ASSERT_EQ(h.frames.size(), 2u);
    EXPECT_EQ(h.frames[1].frameIndex, 2u);
    EXPECT_EQ(h.idrRequests, 0);
}

TEST(FrameAssembler, IdrRequestsAreRateLimited) {
    Harness h;
    Sender tx;
    h.feed(tx.bulk(1, false, bytes(9, 1)));
    h.feed(tx.bulk(2, false, bytes(9, 2)));
    h.feed(tx.bulk(3, false, bytes(9, 3)));
    EXPECT_EQ(h.idrRequests, 1);
    h.now += FrameAssembler::kIdrRepeat;
    h.feed(tx.bulk(4, false, bytes(9, 4)));
    EXPECT_EQ(h.idrRequests, 2) << "still no keyframe: ask again";
}

TEST(FrameAssembler, SequenceGapsAreCountedAsLostPackets) {
    Harness h;
    Sender tx;
    auto packets = tx.bulk(1, true, bytes(40, 1), /*parity=*/0); // 5 packets, seq 100..104
    h.feed(packets[0]);
    h.feed(packets[1]);
    h.feed(packets[4]); // 102 and 103 skipped
    h.feed(packets[2]); // reordered: not lost twice
    const auto s = h.stats.takeSnapshot(1000);
    EXPECT_EQ(s.packetsReceived, 4u);
    EXPECT_EQ(s.packetsLost, 2u);
}

TEST(FrameAssembler, ResetStartsANewSession) {
    Harness h;
    Sender tx;
    h.feed(tx.bulk(50, true, bytes(9, 1)));
    h.assembler.reset();
    // The next session's frame indices may be lower, and it starts without
    // a keyframe until one arrives.
    h.feed(tx.bulk(3, false, bytes(9, 2)));
    h.feed(tx.bulk(4, true, bytes(9, 3)));
    ASSERT_EQ(h.frames.size(), 2u);
    EXPECT_EQ(h.frames[1].frameIndex, 4u);
    EXPECT_EQ(h.assembler.framesLost(), 0u);
}
