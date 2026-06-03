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
    // Must match Rust PROTOCOL_VERSION = 3.
    EXPECT_EQ(PROTOCOL_VERSION, 3);
}

TEST(ClientProtocol, BuildHelloPayloadAdvertisesVersionAndCaps) {
    auto p = buildHelloPayload(PROTOCOL_VERSION, hello_caps::RESOLUTION_SCALE);
    // Layout mirrors Rust encode_hello(): [ver_lo, ver_hi, caps].
    ASSERT_EQ(p.size(), 3u);
    EXPECT_EQ(p[0], 3);     // version low byte (v3)
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
