// Host-buildable unit tests for the Android client's hardware-independent
// control-protocol logic. These compile with a host toolchain (no Android NDK,
// mbedtls, or OpenXR) so client wire-format logic can be regression-tested
// without a device. Wire formats must match the Rust side
// (rust/common/src/protocol.rs).
#include <gtest/gtest.h>

#include <cstdint>
#include <cstring>
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
    // Must match Rust PROTOCOL_VERSION = 6 (each frame's render pose),
    // protocol::stereo_layout, hello_caps and frame_pose::LEN.
    EXPECT_EQ(PROTOCOL_VERSION, 6);
    EXPECT_EQ(hello_caps::FRAME_POSE, 0x02);
    EXPECT_EQ(FRAME_POSE_LEN, 20u);
    EXPECT_EQ(STEREO_MONO, 0);
    EXPECT_EQ(STEREO_SIDE_BY_SIDE, 1);
}

TEST(ClientProtocol, BuildHelloPayloadAdvertisesVersionAndCaps) {
    auto p = buildHelloPayload(PROTOCOL_VERSION, hello_caps::RESOLUTION_SCALE);
    // Layout mirrors Rust encode_hello(): [ver_lo, ver_hi, caps].
    ASSERT_EQ(p.size(), 3u);
    EXPECT_EQ(p[0], 6);     // version low byte (v6)
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
    EXPECT_EQ(c.layout, STEREO_MONO) << "a server before v5 sends one image";
}

TEST(ClientProtocol, ParseStreamConfig26ByteReadsTheStereoLayout) {
    std::vector<uint8_t> p;
    put_u32(p, 1832); put_u32(p, 1920);
    put_u32(p, 80);   put_u32(p, 90);
    p.push_back(1);
    put_u32(p, 1832); put_u32(p, 1920);
    p.push_back(STEREO_SIDE_BY_SIDE);
    ASSERT_EQ(p.size(), 26u);

    StreamConfigView c;
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_EQ(c.encodedWidth, 1832u) << "encoded dims stay per eye";
    EXPECT_EQ(c.layout, STEREO_SIDE_BY_SIDE);

    p[25] = 7;  // unknown layout: show it as one image rather than guess
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_EQ(c.layout, STEREO_MONO);
}

TEST(ClientProtocol, ParseStreamConfig27ByteSaysFramesCarryTheirPose) {
    std::vector<uint8_t> p;
    put_u32(p, 1832); put_u32(p, 1920);
    put_u32(p, 80);   put_u32(p, 90);
    p.push_back(1);
    put_u32(p, 1832); put_u32(p, 1920);
    p.push_back(STEREO_SIDE_BY_SIDE);
    StreamConfigView c;
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_FALSE(c.framePose) << "a v5 server sends no render pose";
    p.push_back(1);
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_TRUE(c.framePose);
    p[26] = 0;
    ASSERT_TRUE(parseStreamConfig(p.data(), p.size(), c));
    EXPECT_FALSE(c.framePose);
}

namespace {
// What Rust protocol::frame_pose::encode writes.
std::vector<uint8_t> framePosePrefix(const float* q) {
    std::vector<uint8_t> p = {'F', 'P', 1, static_cast<uint8_t>(q ? 1 : 0)};
    for (int i = 0; i < 4; i++) {
        uint8_t b[4] = {};
        if (q) std::memcpy(b, &q[i], 4);
        p.insert(p.end(), b, b + 4);
    }
    return p;
}
}  // namespace

TEST(ClientProtocol, ParseFramePoseReadsTheOrientationAndSkipsThePrefix) {
    const float q[4] = {0.1f, -0.2f, 0.3f, 0.927f};
    std::vector<uint8_t> frame = framePosePrefix(q);
    const uint8_t nal[] = {0, 0, 0, 1, 0x40};
    frame.insert(frame.end(), nal, nal + sizeof nal);

    FramePose pose;
    ASSERT_EQ(parseFramePose(frame.data(), frame.size(), pose), FRAME_POSE_LEN);
    EXPECT_TRUE(pose.known);
    for (int i = 0; i < 4; i++) EXPECT_EQ(pose.orientation[i], q[i]);
    EXPECT_EQ(frame[FRAME_POSE_LEN + 3], 1) << "the NAL follows";

    const std::vector<uint8_t> unknown = framePosePrefix(nullptr);
    ASSERT_EQ(parseFramePose(unknown.data(), unknown.size(), pose), FRAME_POSE_LEN);
    EXPECT_FALSE(pose.known);
}

TEST(ClientProtocol, ParseFramePoseRejectsWhatIsNotOne) {
    FramePose pose;
    const uint8_t bareNal[] = {0, 0, 0, 1, 0x40, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};
    EXPECT_EQ(parseFramePose(bareNal, sizeof bareNal, pose), 0u);
    std::vector<uint8_t> p = framePosePrefix(nullptr);
    EXPECT_EQ(parseFramePose(p.data(), 19, pose), 0u) << "short";
    p[2] = 9;
    EXPECT_EQ(parseFramePose(p.data(), p.size(), pose), 0u) << "unknown version";
    const float nan[4] = {std::nanf(""), 0.0f, 0.0f, 1.0f};
    p = framePosePrefix(nan);
    ASSERT_EQ(parseFramePose(p.data(), p.size(), pose), FRAME_POSE_LEN);
    EXPECT_FALSE(pose.known) << "not finite: unknown";
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

TEST(ClientProtocol, ViewConfigPayloadMatchesTheEngine) {
    // Rust encode_view_config / view_config_byte_layout: f32 LE, left eye
    // (left, right, up, down), right eye, IPD.
    const EyeFov left{-0.9f, 0.8f, 0.7f, -0.85f};
    const EyeFov right{-0.8f, 0.9f, 0.7f, -0.85f};
    const auto p = buildViewConfigPayload(left, right, 0.06354f);
    ASSERT_EQ(p.size(), 36u);
    auto f32At = [&](size_t offset) {
        float v;
        std::memcpy(&v, p.data() + offset, 4);  // the host tests run little-endian
        return v;
    };
    EXPECT_FLOAT_EQ(f32At(0), -0.9f) << "left eye, angle left first";
    EXPECT_FLOAT_EQ(f32At(4), 0.8f);
    EXPECT_FLOAT_EQ(f32At(8), 0.7f) << "up before down";
    EXPECT_FLOAT_EQ(f32At(12), -0.85f);
    EXPECT_FLOAT_EQ(f32At(16), -0.8f) << "then the right eye";
    EXPECT_FLOAT_EQ(f32At(32), 0.0635f) << "IPD, rounded to 0.1 mm";
}

TEST(ClientProtocol, ViewConfigIgnoresIpdNoiseBelowATenthOfAMillimetre) {
    const EyeFov eye{-0.9f, 0.9f, 0.8f, -0.8f};
    EXPECT_EQ(buildViewConfigPayload(eye, eye, 0.063501f), buildViewConfigPayload(eye, eye, 0.063498f));
    EXPECT_NE(buildViewConfigPayload(eye, eye, 0.0635f), buildViewConfigPayload(eye, eye, 0.0637f));
}

TEST(ClientProtocol, DecoderInitDimsHoldBothEyesSideBySide) {
    // REGRESSION (mono): the decoder was sized for one eye, and the one
    // image went to both eyes.
    auto d = decoderInitDims(1832, 1920, 1832, 1920, STEREO_SIDE_BY_SIDE);
    EXPECT_EQ(d.width, 3664u);
    EXPECT_EQ(d.height, 1920u);
    d = decoderInitDims(1832, 1920, 916, 960, STEREO_SIDE_BY_SIDE);
    EXPECT_EQ(d.width, 1832u) << "two downscaled eyes";
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

// ---------------------------------------------------------------------------
// HEARTBEAT / HEARTBEAT_ACK / HAPTIC_EVENT (engine.rs handle_tcp_control)
// ---------------------------------------------------------------------------

TEST(Heartbeat, PayloadLayoutMatchesTheEngine) {
    HeartbeatStats s;
    s.packetsReceived = 0x01020304;
    s.packetsLost = 7;
    s.avgDecodeUs = 2500;
    s.fps = 90;
    const auto p = buildHeartbeatPayload(0xAABBCCDD, 0x1122334455667788ULL, s);
    ASSERT_EQ(p.size(), 26u) << "the engine ignores heartbeats shorter than 26 bytes";
    EXPECT_EQ(readU32Le(p.data()), 0xAABBCCDDu);
    EXPECT_EQ(p[4], 0x88);
    EXPECT_EQ(p[11], 0x11);
    // The engine reads the stats at offset 12.
    EXPECT_EQ(readU32Le(p.data() + 12), 0x01020304u);
    EXPECT_EQ(readU32Le(p.data() + 16), 7u);
    EXPECT_EQ(readU32Le(p.data() + 20), 2500u);
    EXPECT_EQ(readU16Le(p.data() + 24), 90u);
}

TEST(Heartbeat, AckCarriesPcLatencies) {
    const uint8_t payload[8] = {0x10, 0x27, 0, 0, 0x20, 0x4E, 0, 0}; // 10000, 20000
    HeartbeatAck ack;
    ASSERT_TRUE(parseHeartbeatAck(payload, sizeof(payload), ack));
    EXPECT_EQ(ack.pcEncodeUs, 10000u);
    EXPECT_EQ(ack.pcTotalUs, 20000u);
    EXPECT_FALSE(parseHeartbeatAck(payload, 7, ack));
}

TEST(Haptic, ParsesTheEnginePayload) {
    // engine.rs HapticEvent::to_payload: id u8, duration_ms u16, frequency f32, amplitude f32.
    uint8_t payload[11] = {1, 0x2C, 0x01}; // controller 1, 300 ms
    const float frequency = 160.0f;
    const float amplitude = 0.75f;
    std::memcpy(payload + 3, &frequency, 4);
    std::memcpy(payload + 7, &amplitude, 4);
    HapticEvent e;
    ASSERT_TRUE(parseHapticEvent(payload, sizeof(payload), e));
    EXPECT_EQ(e.controllerId, 1);
    EXPECT_EQ(e.durationMs, 300);
    EXPECT_FLOAT_EQ(e.frequency, 160.0f);
    EXPECT_FLOAT_EQ(e.amplitude, 0.75f);
    EXPECT_FALSE(parseHapticEvent(payload, 10, e));
}

TEST(Rtp, SequenceNumberIsBigEndian) {
    const uint8_t header[4] = {0x80, 97, 0x12, 0x34};
    EXPECT_EQ(rtpSequence(header), 0x1234);
}
