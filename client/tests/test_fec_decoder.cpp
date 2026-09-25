// Host tests for the client's Reed-Solomon FEC decoder (fec_decoder.cpp),
// built with a desktop toolchain against the stubs in client/tests/shim.
//
// Parity bytes are cross-language golden vectors: produced by the server's
// encoder (Rust reed-solomon-erasure) and pinned on that side by
// rust/streaming-engine/src/transport/fec.rs
// `test_fec_golden_parity_matches_client_fixture`. If either side's matrix
// construction drifts, one of the two tests fails.
#include <gtest/gtest.h>

#include <cstdint>
#include <initializer_list>
#include <optional>
#include <vector>

#include "fec_decoder.h"

namespace {

constexpr int kData = 10;
constexpr int kShardLen = 8;

// Parity rows for the golden data below. Row k is identical for parity
// counts 1 / 2 / 4 (redundancy 0.1 / 0.2 / 0.4), so each config uses a prefix.
const uint8_t kGoldenParity[4][kShardLen] = {
    {0x89, 0x4B, 0xA8, 0xC9, 0xAF, 0x9C, 0xA2, 0xB2},
    {0x09, 0x9C, 0x7D, 0x5A, 0xB1, 0x6F, 0xAD, 0x8E},
    {0xC5, 0x35, 0x6B, 0xE8, 0xB1, 0xEC, 0xAE, 0x95},
    {0x3A, 0xE8, 0x86, 0xFC, 0x06, 0x29, 0xB3, 0xD6},
};

// Data shard i, byte j = (i*37 + j*11 + 5) & 0xFF — same formula as Rust.
std::vector<uint8_t> goldenShard(int index) {
    std::vector<uint8_t> s(kShardLen);
    for (int j = 0; j < kShardLen; j++) {
        s[j] = index < kData ? static_cast<uint8_t>((index * 37 + j * 11 + 5) & 0xFF)
                             : kGoldenParity[index - kData][j];
    }
    return s;
}

std::vector<uint8_t> goldenFrame() {
    std::vector<uint8_t> out;
    for (int i = 0; i < kData; i++) {
        auto s = goldenShard(i);
        out.insert(out.end(), s.begin(), s.end());
    }
    return out;
}

bool contains(const std::vector<int>& v, int x) {
    for (int e : v) if (e == x) return true;
    return false;
}

// Begin a frame of kData + parity shards, claiming `dataShards` of them are
// data, feed every shard except `lost`, and try to decode.
std::optional<FecFrameDecoder::DecodedFrame> decodeGolden(
        FecFrameDecoder& dec, int parity, uint16_t dataShards, const std::vector<int>& lost) {
    const uint16_t total = static_cast<uint16_t>(kData + parity);
    if (!dec.beginFrame(42, total, dataShards, true)) return std::nullopt;
    for (int i = 0; i < total; i++) {
        if (contains(lost, i)) continue;
        auto s = goldenShard(i);
        dec.addShard(static_cast<uint16_t>(i), s.data(), static_cast<int>(s.size()));
    }
    return dec.tryDecode();
}

}  // namespace

TEST(FecFrameDecoder, RecoversMaxLossWithHeaderDataCount) {
    // Lose as many data shards as there are parity shards — the most RS can
    // recover — at each redundancy adaptive FEC may pick.
    for (int parity : {1, 2, 4}) {
        std::vector<int> lost;
        for (int k = 0; k < parity; k++) lost.push_back(k * 3);  // 0, 3, 6, 9
        FecFrameDecoder dec;
        auto frame = decodeGolden(dec, parity, kData, lost);
        ASSERT_TRUE(frame.has_value()) << "parity " << parity;
        EXPECT_EQ(frame->data, goldenFrame()) << "parity " << parity;
        EXPECT_EQ(frame->frameIndex, 42u);
        EXPECT_TRUE(frame->isKeyframe);
    }
}

TEST(FecFrameDecoder, AllDataPresentSkipsReconstruction) {
    FecFrameDecoder dec;
    auto frame = decodeGolden(dec, 4, kData, {10, 11, 12, 13});  // only parity lost
    ASSERT_TRUE(frame.has_value());
    EXPECT_EQ(frame->data, goldenFrame());
}

TEST(FecFrameDecoder, LegacyTotalOverOnePointTwoGuessFails) {
    // REGRESSION for the old `dataShards = totalShards / 1.2` guess.
    // 40%: total 14 → guess 11 > 10 — it waits for 11 shards and drops a frame
    // that 10 shards (4 lost) can fully recover.
    {
        const uint16_t guess = static_cast<uint16_t>(14 / 1.2f);
        ASSERT_EQ(guess, 11);
        FecFrameDecoder dec;
        EXPECT_FALSE(decodeGolden(dec, 4, guess, {0, 3, 6, 9}).has_value());
        FecFrameDecoder fixed;
        EXPECT_TRUE(decodeGolden(fixed, 4, kData, {0, 3, 6, 9}).has_value());
    }
    // 10%: total 11 → guess 9 < 10 — RS runs on the wrong code and "recovers"
    // garbage.
    {
        const uint16_t guess = static_cast<uint16_t>(11 / 1.2f);
        ASSERT_EQ(guess, 9);
        FecFrameDecoder dec;
        auto frame = decodeGolden(dec, 1, guess, {0});
        ASSERT_TRUE(frame.has_value());
        EXPECT_NE(frame->data, goldenFrame());
    }
}

TEST(FecFrameDecoder, RejectsInvalidCounts) {
    // Counts come from the UDP header: data == 0 used to report "complete"
    // immediately; data > total read m_received[] out of range in tryDecode.
    struct Case { uint16_t total, data; };
    for (Case c : {Case{14, 0}, Case{14, 15}, Case{0, 0},
                   Case{FecFrameDecoder::MAX_TOTAL_SHARDS + 1, 1}}) {
        FecFrameDecoder dec;
        EXPECT_FALSE(dec.beginFrame(1, c.total, c.data, false))
            << "total " << c.total << " data " << c.data;
        auto s = goldenShard(0);
        dec.addShard(0, s.data(), static_cast<int>(s.size()));
        EXPECT_FALSE(dec.isComplete());
        EXPECT_FALSE(dec.tryDecode().has_value());
        EXPECT_FALSE(dec.isActiveFor(1));
    }
}

TEST(FecFrameDecoder, ValidCountsAfterRejectionWork) {
    FecFrameDecoder dec;
    EXPECT_FALSE(dec.beginFrame(1, 14, 15, false));
    auto frame = decodeGolden(dec, 2, kData, {5});
    ASSERT_TRUE(frame.has_value());
    EXPECT_EQ(frame->data, goldenFrame());
}

TEST(FecFrameDecoder, DeliversEachFrameOnce) {
    // The render loop polls isComplete()/tryDecode() every frame; a completed
    // frame must not be resubmitted to MediaCodec on every poll, nor again when
    // late parity shards arrive.
    FecFrameDecoder dec;
    ASSERT_TRUE(decodeGolden(dec, 2, kData, {}).has_value());
    EXPECT_TRUE(dec.isDelivered());
    EXPECT_FALSE(dec.tryDecode().has_value());
    auto late = goldenShard(11);
    dec.addShard(11, late.data(), static_cast<int>(late.size()));
    EXPECT_FALSE(dec.tryDecode().has_value());
}

TEST(FecFrameDecoder, FrameIndexZeroIsNotMistakenForStarted) {
    // A fresh decoder's currentFrameIndex() is 0, so the old
    // `frameIndex != currentFrameIndex()` check never began frame 0.
    FecFrameDecoder dec;
    EXPECT_FALSE(dec.isActiveFor(0));
    ASSERT_TRUE(dec.beginFrame(0, 2, 1, false));
    EXPECT_TRUE(dec.isActiveFor(0));
    EXPECT_FALSE(dec.isActiveFor(1));
}

// --- SlicedFecFrameDecoder ---

namespace {
// One slice: data shards carrying [u32 LE length][payload], zero padded.
std::vector<std::vector<uint8_t>> sliceShards(const std::vector<uint8_t>& payload, int shardLen) {
    std::vector<uint8_t> prefixed;
    const uint32_t len = static_cast<uint32_t>(payload.size());
    for (int b = 0; b < 4; b++) prefixed.push_back(static_cast<uint8_t>((len >> (8 * b)) & 0xFF));
    prefixed.insert(prefixed.end(), payload.begin(), payload.end());
    std::vector<std::vector<uint8_t>> shards;
    for (size_t off = 0; off < prefixed.size(); off += shardLen) {
        std::vector<uint8_t> s(shardLen, 0);
        for (int j = 0; j < shardLen && off + j < prefixed.size(); j++) s[j] = prefixed[off + j];
        shards.push_back(s);
    }
    return shards;
}
}  // namespace

TEST(SlicedFecFrameDecoder, NotCompleteBeforeFirstFrame) {
    // REGRESSION: with sliceCount 0 the empty completion bitmask compared
    // equal, so a decoder that never saw a sliced frame reported "complete"
    // and the render loop pushed an empty frame (→ NAL reject → IDR request +
    // decoder flush) on every iteration while bulk frames were streaming.
    SlicedFecFrameDecoder dec;
    EXPECT_FALSE(dec.isComplete());
    EXPECT_FALSE(dec.tryDecode().has_value());
    EXPECT_FALSE(dec.isActive());
}

TEST(SlicedFecFrameDecoder, AssemblesSlicesStripsPrefixAndDeliversOnce) {
    const std::vector<uint8_t> a = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11};
    const std::vector<uint8_t> b = {20, 21, 22};
    auto sa = sliceShards(a, 8);  // 15 bytes → 2 data shards
    auto sb = sliceShards(b, 8);  // 7 bytes → 1 data shard
    ASSERT_EQ(sa.size(), 2u);
    ASSERT_EQ(sb.size(), 1u);

    SlicedFecFrameDecoder dec;
    dec.beginFrame(0, 2, true);  // frame index 0 must work too
    // Each slice has its own counts; a parity shard slot exists but is never
    // sent (all data arrives → no reconstruction needed).
    dec.addShard(0, 0, 3, 2, sa[0].data(), 8);
    EXPECT_FALSE(dec.isComplete());
    dec.addShard(0, 1, 3, 2, sa[1].data(), 8);
    dec.addShard(1, 0, 2, 1, sb[0].data(), 8);
    ASSERT_TRUE(dec.isComplete());

    auto frame = dec.tryDecode();
    ASSERT_TRUE(frame.has_value());
    std::vector<uint8_t> expected = a;
    expected.insert(expected.end(), b.begin(), b.end());
    EXPECT_EQ(frame->data, expected);
    EXPECT_FALSE(dec.tryDecode().has_value()) << "frame must be delivered once";
}

TEST(SlicedFecFrameDecoder, SliceCountChangesBetweenFrames) {
    // The server picks the slice count per frame: a large IDR gets more
    // slices than the configured count so each slice fits one RS code word,
    // up to the 4-bit maximum of 15. One decoder must follow 2 → 15 → 3.
    SlicedFecFrameDecoder dec;
    uint32_t frameIndex = 0;
    for (int slices : {2, SlicedFecFrameDecoder::MAX_SLICES, 3}) {
        dec.beginFrame(frameIndex, static_cast<uint8_t>(slices), false);
        std::vector<uint8_t> expected;
        for (int s = 0; s < slices; s++) {
            // 5 bytes + 4-byte prefix → 2 data shards of 8; parity never sent.
            std::vector<uint8_t> payload(5, static_cast<uint8_t>(frameIndex * 16 + s));
            auto shards = sliceShards(payload, 8);
            ASSERT_EQ(shards.size(), 2u);
            for (uint16_t i = 0; i < 2; i++) {
                dec.addShard(static_cast<uint8_t>(s), i, 3, 2, shards[i].data(), 8);
            }
            expected.insert(expected.end(), payload.begin(), payload.end());
        }
        ASSERT_TRUE(dec.isComplete()) << slices << " slices";
        auto frame = dec.tryDecode();
        ASSERT_TRUE(frame.has_value()) << slices << " slices";
        EXPECT_EQ(frame->frameIndex, frameIndex);
        EXPECT_EQ(frame->data, expected) << slices << " slices";
        frameIndex++;
    }
}

TEST(SlicedFecFrameDecoder, RejectsSliceWithInvalidCounts) {
    const std::vector<uint8_t> a = {1, 2, 3};
    auto sa = sliceShards(a, 8);
    SlicedFecFrameDecoder dec;
    dec.beginFrame(5, 1, false);
    dec.addShard(0, 0, 2, 3, sa[0].data(), 8);  // data > total
    EXPECT_FALSE(dec.isComplete());
    EXPECT_FALSE(dec.tryDecode().has_value());
}

TEST(SlicedFecFrameDecoder, AbandonStopsFurtherTimeoutsAndShards) {
    const std::vector<uint8_t> a = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11};
    auto sa = sliceShards(a, 8);
    SlicedFecFrameDecoder dec;
    dec.beginFrame(9, 1, false);
    dec.addShard(0, 0, 3, 2, sa[0].data(), 8);
    EXPECT_TRUE(dec.isActive());
    dec.abandon();
    EXPECT_FALSE(dec.isActive());
    dec.addShard(0, 1, 3, 2, sa[1].data(), 8);
    EXPECT_FALSE(dec.isComplete());
    EXPECT_FALSE(dec.tryDecode().has_value());
}
