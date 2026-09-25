// Host-buildable unit tests for the Android client's hardware-independent
// control-protocol logic. These compile with a host toolchain (no Android NDK,
// mbedtls, or OpenXR) so client wire-format logic can be regression-tested
// without a device. Wire formats must match the Rust side
// (rust/common/src/protocol.rs).
#include <gtest/gtest.h>

#include <cstdint>
#include <vector>

#include "client_protocol.h"

using namespace fvp_client_protocol;

namespace {
void put_u32(std::vector<uint8_t>& v, uint32_t x) {
    v.push_back(static_cast<uint8_t>(x & 0xFF));
    v.push_back(static_cast<uint8_t>((x >> 8) & 0xFF));
    v.push_back(static_cast<uint8_t>((x >> 16) & 0xFF));
    v.push_back(static_cast<uint8_t>((x >> 24) & 0xFF));
}
}  // namespace

TEST(ClientProtocol, ProtocolVersionMatchesServer) {
    // Must match Rust PROTOCOL_VERSION = 4 (12-byte FVP header with
    // data_shard_count).
    EXPECT_EQ(PROTOCOL_VERSION, 4);
}

TEST(ClientProtocol, BuildHelloPayloadAdvertisesVersionAndCaps) {
    auto p = buildHelloPayload(PROTOCOL_VERSION, hello_caps::RESOLUTION_SCALE);
    // Layout mirrors Rust encode_hello(): [ver_lo, ver_hi, caps].
    ASSERT_EQ(p.size(), 3u);
    EXPECT_EQ(p[0], 4);     // version low byte (v4)
    EXPECT_EQ(p[1], 0);     // version high byte
    EXPECT_EQ(p[2], 0x01);  // RESOLUTION_SCALE
}

TEST(ClientProtocol, BuildHelloPayloadVersionIsLittleEndian) {
    auto p = buildHelloPayload(0x0102, 0x00);
    EXPECT_EQ(p[0], 0x02);  // low byte first
    EXPECT_EQ(p[1], 0x01);  // high byte second
    EXPECT_EQ(p[2], 0x00);  // no caps
}

TEST(ClientProtocol, ParseStreamConfig25ByteReadsEncodedDims) {
    std::vector<uint8_t> p;
    put_u32(p, 1832); put_u32(p, 1920);  // native render (target) resolution
    put_u32(p, 80);   put_u32(p, 90);    // bitrate_mbps, framerate
    p.push_back(1);                       // codec = h265
    put_u32(p, 916);  put_u32(p, 960);   // encoded (downscaled) dimensions
    ASSERT_EQ(p.size(), 25u);

    StreamConfigView c;
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_EQ(c.width, 1832u);
    EXPECT_EQ(c.height, 1920u);
    EXPECT_EQ(c.bitrateMbps, 80u);
    EXPECT_EQ(c.framerate, 90u);
    EXPECT_EQ(c.codec, 1);
    EXPECT_EQ(c.encodedWidth, 916u);
    EXPECT_EQ(c.encodedHeight, 960u);
}

TEST(ClientProtocol, ParseStreamConfigLegacy17ByteEncodedEqualsNative) {
    // Old server (no encoded dims): decode at native resolution — REGRESSION.
    std::vector<uint8_t> p;
    put_u32(p, 1832); put_u32(p, 1920);
    put_u32(p, 80);   put_u32(p, 90);
    p.push_back(1);
    ASSERT_EQ(p.size(), 17u);

    StreamConfigView c;
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_EQ(c.encodedWidth, 1832u);
    EXPECT_EQ(c.encodedHeight, 1920u);
}

TEST(ClientProtocol, ParseStreamConfigRejectsShortPayload) {
    std::vector<uint8_t> p(16, 0);  // below the 17-byte minimum
    StreamConfigView c;
    EXPECT_FALSE(parseStreamConfig(p.data(), p.size(), c));
}

TEST(ClientProtocol, DecoderInitDimsUsesEncodedWhenKnown) {
    auto d = decoderInitDims(1832, 1920, 916, 960);
    EXPECT_EQ(d.width, 916u);
    EXPECT_EQ(d.height, 960u);
}

TEST(ClientProtocol, DecoderInitDimsFallsBackToNativeWhenEncodedUnset) {
    // Before STREAM_CONFIG arrives (or a legacy server), encoded is 0 — decode
    // at native resolution.
    auto d = decoderInitDims(1832, 1920, 0, 0);
    EXPECT_EQ(d.width, 1832u);
    EXPECT_EQ(d.height, 1920u);
}

// --- FVP video packet header (v4) ---

namespace {
// Build RTP (12 zero bytes — not parsed here) + FVP header + payload,
// matching Rust write_rtp_header/write_fvp_header.
std::vector<uint8_t> makePacket(uint32_t frame, uint16_t shardIndex, uint16_t total,
                                uint16_t flags, uint16_t data, size_t payloadLen = 4) {
    std::vector<uint8_t> v(RTP_HEADER_LEN, 0);
    put_u32(v, frame);
    auto put_u16 = [&v](uint16_t x) {
        v.push_back(static_cast<uint8_t>(x & 0xFF));
        v.push_back(static_cast<uint8_t>(x >> 8));
    };
    put_u16(shardIndex);
    put_u16(total);
    put_u16(flags);
    put_u16(data);
    v.insert(v.end(), payloadLen, 0xAB);
    return v;
}
}  // namespace

TEST(FvpHeader, LayoutConstantsMatchServer) {
    // Rust: RTP_HEADER_LEN 12, FVP_HEADER_LEN 12, PACKET_HEADER_LEN 24.
    EXPECT_EQ(RTP_HEADER_LEN, 12u);
    EXPECT_EQ(FVP_HEADER_LEN, 12u);
    EXPECT_EQ(PACKET_HEADER_LEN, 24u);
}

TEST(FvpHeader, ParsesAllFieldsLittleEndian) {
    // Same bytes as Rust test_fvp_header_byte_layout.
    auto p = makePacket(0x04030201, 0x0605, 0x0807, 0x0A09, 0x0605);
    FvpHeaderView h;
    ASSERT_TRUE(parseFvpHeader(p.data(), p.size(), h));
    EXPECT_EQ(h.frameIndex, 0x04030201u);
    EXPECT_EQ(h.shardIndex, 0x0605);
    EXPECT_EQ(h.totalShards, 0x0807);
    EXPECT_EQ(h.flags, 0x0A09);
    EXPECT_EQ(h.dataShards, 0x0605);
    EXPECT_EQ(p[22], 0x05);  // data_shard_count sits at [22..24]
    EXPECT_EQ(p[23], 0x06);
}

TEST(FvpHeader, DataShardsComeFromHeaderNotTotalGuess) {
    // REGRESSION: 10 data + 4 parity (40% redundancy). The old client guessed
    // total / 1.2 = 11 — the header now carries the real 10.
    auto p = makePacket(7, 13, 14, 0, 10);
    FvpHeaderView h;
    ASSERT_TRUE(parseFvpHeader(p.data(), p.size(), h));
    EXPECT_EQ(h.totalShards, 14);
    EXPECT_EQ(h.dataShards, 10);
    EXPECT_NE(static_cast<uint16_t>(h.totalShards / 1.2f), h.dataShards);
}

TEST(FvpHeader, AcceptsBoundaryCounts) {
    FvpHeaderView h;
    auto one = makePacket(0, 0, 1, 0, 1);  // single data shard, no parity
    EXPECT_TRUE(parseFvpHeader(one.data(), one.size(), h));
    auto max = makePacket(0, MAX_FRAME_SHARDS - 1, MAX_FRAME_SHARDS, 0, MAX_FRAME_SHARDS);
    EXPECT_TRUE(parseFvpHeader(max.data(), max.size(), h));
    // Header-only packet (no payload) still parses; payload checks are the
    // decoder's job.
    auto bare = makePacket(0, 0, 2, 0, 1, 0);
    EXPECT_TRUE(parseFvpHeader(bare.data(), bare.size(), h));
}

TEST(FvpHeader, RejectsInconsistentShardFields) {
    FvpHeaderView h;
    auto dataZero = makePacket(0, 0, 14, 0, 0);
    EXPECT_FALSE(parseFvpHeader(dataZero.data(), dataZero.size(), h));
    auto dataOverTotal = makePacket(0, 0, 14, 0, 15);
    EXPECT_FALSE(parseFvpHeader(dataOverTotal.data(), dataOverTotal.size(), h));
    auto indexOutOfRange = makePacket(0, 14, 14, 0, 10);
    EXPECT_FALSE(parseFvpHeader(indexOutOfRange.data(), indexOutOfRange.size(), h));
    auto totalZero = makePacket(0, 0, 0, 0, 0);
    EXPECT_FALSE(parseFvpHeader(totalZero.data(), totalZero.size(), h));
    auto totalTooBig = makePacket(0, 0, MAX_FRAME_SHARDS + 1, 0, 1);
    EXPECT_FALSE(parseFvpHeader(totalTooBig.data(), totalTooBig.size(), h));
}

TEST(FvpHeader, RejectsShortPackets) {
    FvpHeaderView h;
    auto p = makePacket(1, 0, 2, 0, 1, 0);
    // A v3-sized (22-byte) header lacks data_shard_count.
    EXPECT_FALSE(parseFvpHeader(p.data(), 22, h));
    EXPECT_FALSE(parseFvpHeader(p.data(), PACKET_HEADER_LEN - 1, h));
    EXPECT_FALSE(parseFvpHeader(nullptr, 100, h));
}

TEST(FvpHeader, RejectedParseLeavesOutputUntouched) {
    FvpHeaderView h;
    h.frameIndex = 99;
    auto bad = makePacket(5, 0, 3, 0, 4);
    EXPECT_FALSE(parseFvpHeader(bad.data(), bad.size(), h));
    EXPECT_EQ(h.frameIndex, 99u);
}
