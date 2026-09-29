#pragma once
// Hardware-independent control-protocol helpers for the Android client.
//
// Kept free of Android / mbedtls / OpenXR dependencies so the logic is
// host-buildable and unit-testable (see client/tests/). Wire formats here MUST
// match the Rust side (rust/common/src/protocol.rs).
#include <array>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstring>

namespace fvp_client_protocol {

// Protocol version — must match Rust PROTOCOL_VERSION. The client implements the
// v5 wire format: v3 FVP slice/stream flags (see fec_decoder.h fvp_flags), the
// 12-byte FVP header carrying data_shard_count (parseFvpHeader below, v4), and
// side-by-side stereo frames (STREAM_CONFIG byte 25, v5).
inline constexpr uint16_t PROTOCOL_VERSION = 5;

// How the eyes are laid out in a video frame — must match Rust
// protocol::stereo_layout.
inline constexpr uint8_t STEREO_MONO = 0;          // one image for both eyes (servers before v5)
inline constexpr uint8_t STEREO_SIDE_BY_SIDE = 1;  // left eye | right eye, 2 × encoded width

// Control message types — must match Rust protocol::msg_type.
namespace msg {
    inline constexpr uint8_t HELLO = 0x01;
    inline constexpr uint8_t HELLO_ACK = 0x02;
    inline constexpr uint8_t PIN_REQUEST = 0x03;
    inline constexpr uint8_t PIN_RESPONSE = 0x04;
    inline constexpr uint8_t PIN_RESULT = 0x05;
    inline constexpr uint8_t STREAM_CONFIG = 0x06;
    inline constexpr uint8_t STREAM_START = 0x07;
    inline constexpr uint8_t HEARTBEAT = 0x10;
    inline constexpr uint8_t HEARTBEAT_ACK = 0x11;
    inline constexpr uint8_t VIEW_CONFIG = 0x22;
    inline constexpr uint8_t IDR_REQUEST = 0x30;
    inline constexpr uint8_t FACE_DATA = 0x35;
    inline constexpr uint8_t HAPTIC_EVENT = 0x38;
    inline constexpr uint8_t SLEEP_ENTER = 0x50;
    inline constexpr uint8_t SLEEP_EXIT = 0x51;
    inline constexpr uint8_t CONFIG_UPDATE = 0x55;
    inline constexpr uint8_t CONFIG_UPDATE_ACK = 0x56;
    inline constexpr uint8_t DISCONNECT = 0xFF;
}  // namespace msg

// Largest [type + payload] the client accepts from the server; its control
// messages are a few bytes each.
inline constexpr uint32_t MAX_CONTROL_MESSAGE_LEN = 64 * 1024;

// Default UDP base port — must match Rust DEFAULT_UDP_PORT. The engine sends
// video to base+1 and audio to base+3, and receives tracking on base+2.
inline constexpr uint16_t DEFAULT_UDP_BASE_PORT = 9945;
inline constexpr uint16_t VIDEO_PORT_OFFSET = 1;
inline constexpr uint16_t TRACKING_PORT_OFFSET = 2;
inline constexpr uint16_t AUDIO_PORT_OFFSET = 3;

inline void writeU16Le(uint8_t* p, uint16_t v) {
    p[0] = static_cast<uint8_t>(v);
    p[1] = static_cast<uint8_t>(v >> 8);
}

inline void writeU32Le(uint8_t* p, uint32_t v) {
    for (int i = 0; i < 4; i++) p[i] = static_cast<uint8_t>(v >> (8 * i));
}

inline void writeU64Le(uint8_t* p, uint64_t v) {
    for (int i = 0; i < 8; i++) p[i] = static_cast<uint8_t>(v >> (8 * i));
}

inline void writeF32Le(uint8_t* p, float v) {
    uint32_t bits;
    std::memcpy(&bits, &v, sizeof(bits));
    writeU32Le(p, bits);
}

// One eye's field of view as OpenXR reports it (XrFovf): radians from
// straight ahead, `left` and `down` negative.
struct EyeFov {
    float left = 0.0f;
    float right = 0.0f;
    float up = 0.0f;
    float down = 0.0f;
};

inline constexpr size_t VIEW_CONFIG_PAYLOAD_LEN = 36;
using ViewConfigPayload = std::array<uint8_t, VIEW_CONFIG_PAYLOAD_LEN>;

// VIEW_CONFIG payload — must match Rust encode_view_config(): little-endian
// f32 left eye (left, right, up, down), right eye (same), IPD in metres. The
// PC renders SteamVR's views with these. The IPD is rounded to 0.1 mm so that
// tracking noise in the eye positions doesn't make every frame a change.
inline ViewConfigPayload buildViewConfigPayload(const EyeFov& left, const EyeFov& right, float ipdM) {
    ViewConfigPayload p{};
    const float values[9] = {
        left.left, left.right, left.up, left.down,
        right.left, right.right, right.up, right.down,
        std::round(ipdM * 10000.0f) / 10000.0f,
    };
    for (size_t i = 0; i < 9; i++) writeF32Le(p.data() + i * 4, values[i]);
    return p;
}

// HMD statistics carried in every HEARTBEAT, for one report interval.
struct HeartbeatStats {
    uint32_t packetsReceived = 0;
    uint32_t packetsLost = 0;
    uint32_t avgDecodeUs = 0;
    uint16_t fps = 0;
};

inline constexpr size_t HEARTBEAT_PAYLOAD_LEN = 26;

// HEARTBEAT payload: [sequence u32][timestamp_ms u64][received u32][lost u32]
// [avg_decode_us u32][fps u16], little-endian. The engine reads the stats at
// offset 12 (engine.rs handle_tcp_control) and answers with HEARTBEAT_ACK.
inline std::array<uint8_t, HEARTBEAT_PAYLOAD_LEN> buildHeartbeatPayload(
        uint32_t sequence, uint64_t timestampMs, const HeartbeatStats& s) {
    std::array<uint8_t, HEARTBEAT_PAYLOAD_LEN> p{};
    writeU32Le(p.data() + 0, sequence);
    writeU64Le(p.data() + 4, timestampMs);
    writeU32Le(p.data() + 12, s.packetsReceived);
    writeU32Le(p.data() + 16, s.packetsLost);
    writeU32Le(p.data() + 20, s.avgDecodeUs);
    writeU16Le(p.data() + 24, s.fps);
    return p;
}

// HELLO capability flags — must match Rust protocol::hello_caps. An absent caps
// byte (legacy / version-only HELLO) means no capabilities.
namespace hello_caps {
    // The client sizes its decoder from the STREAM_CONFIG encoded dimensions and
    // deliberately handles a sub-native (downscaled) stream. The server only
    // downscales for clients that advertise this bit.
    inline constexpr uint8_t RESOLUTION_SCALE = 0x01;
}  // namespace hello_caps

// Build the HELLO payload: protocol version (u16 LE) followed by a capability
// byte. Mirrors Rust encode_hello(): [ver_lo, ver_hi, caps].
inline std::array<uint8_t, 3> buildHelloPayload(uint16_t version, uint8_t caps) {
    return {
        static_cast<uint8_t>(version & 0xFF),
        static_cast<uint8_t>((version >> 8) & 0xFF),
        caps,
    };
}

// Read a little-endian u32 (endian-safe regardless of host byte order; matches
// Rust to_le_bytes()).
inline uint32_t readU32Le(const uint8_t* p) {
    return static_cast<uint32_t>(p[0])
         | (static_cast<uint32_t>(p[1]) << 8)
         | (static_cast<uint32_t>(p[2]) << 16)
         | (static_cast<uint32_t>(p[3]) << 24);
}

// Parsed STREAM_CONFIG view. `width`/`height` are the native render (target)
// resolution per eye; `encodedWidth`/`encodedHeight` are what is actually
// encoded per eye — equal to native unless the server downscaled
// (resolution_scale). `layout` says how the eyes share a frame.
struct StreamConfigView {
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t bitrateMbps = 0;
    uint32_t framerate = 0;
    uint8_t codec = 0;
    uint32_t encodedWidth = 0;
    uint32_t encodedHeight = 0;
    uint8_t layout = STEREO_MONO;
};

// Parse a STREAM_CONFIG payload. Layout (little-endian) — see Rust
// encode_stream_config():
//   [0..4] render_w | [4..8] render_h | [8..12] bitrate | [12..16] framerate |
//   [16] codec | [17..21] encoded_w | [21..25] encoded_h | [25] stereo layout
// A payload of >= 25 bytes carries explicit encoded dims; a legacy 17..24-byte
// payload (old server) has none, so encoded falls back to native. Without byte
// 25 (a server before v5) the stream is mono. Returns false for a payload
// shorter than the 17-byte minimum.
inline bool parseStreamConfig(const uint8_t* payload, size_t len, StreamConfigView& out) {
    if (len < 17) {
        return false;
    }
    out.width = readU32Le(payload + 0);
    out.height = readU32Le(payload + 4);
    out.bitrateMbps = readU32Le(payload + 8);
    out.framerate = readU32Le(payload + 12);
    out.codec = payload[16];
    if (len >= 25) {
        out.encodedWidth = readU32Le(payload + 17);
        out.encodedHeight = readU32Le(payload + 21);
    } else {
        // Legacy server: no separate encoded dims — decode at native resolution.
        out.encodedWidth = out.width;
        out.encodedHeight = out.height;
    }
    out.layout = len >= 26 && payload[25] == STEREO_SIDE_BY_SIDE ? STEREO_SIDE_BY_SIDE : STEREO_MONO;
    return true;
}

// --- Video packet header (RTP + FVP) — must match Rust transport/rtp.rs ---
inline constexpr size_t RTP_HEADER_LEN = 12;
inline constexpr size_t FVP_HEADER_LEN = 12;
// Payload (one FEC shard) starts right after both headers.
inline constexpr size_t PACKET_HEADER_LEN = RTP_HEADER_LEN + FVP_HEADER_LEN;
// Upper bound on shards (data + parity) per frame or slice — Rust
// MAX_FRAME_SHARDS. Larger counts are rejected before any allocation.
inline constexpr uint16_t MAX_FRAME_SHARDS = 4096;

inline uint16_t readU16Le(const uint8_t* p) {
    return static_cast<uint16_t>(p[0] | (p[1] << 8));
}

// Parsed FVP header. `totalShards` counts data + parity for the frame (or for
// the slice when fvp_flags::sliceCount(flags) > 0); `dataShards` says how many
// of them are data. Adaptive FEC varies the parity ratio per frame, so the
// data count must come from the header — it cannot be derived from the total.
struct FvpHeaderView {
    uint32_t frameIndex = 0;
    uint16_t shardIndex = 0;
    uint16_t totalShards = 0;
    uint16_t flags = 0;
    uint16_t dataShards = 0;
};

// Parse and validate the FVP header of a received video packet. Layout
// (little-endian), offsets from the start of the UDP payload:
//   [12..16] frame_index | [16..18] shard_index | [18..20] shard_count |
//   [20..22] flags | [22..24] data_shard_count (v4) | [24..] shard payload
// Returns false — drop the packet — when it is too short or the shard fields
// are inconsistent: shard_count must be 1..MAX_FRAME_SHARDS, shard_index <
// shard_count, and data_shard_count 1..shard_count. These fields size and
// index the FEC decoder's buffers, so they must never be trusted unchecked.
inline bool parseFvpHeader(const uint8_t* packet, size_t len, FvpHeaderView& out) {
    if (packet == nullptr || len < PACKET_HEADER_LEN) {
        return false;
    }
    const uint8_t* f = packet + RTP_HEADER_LEN;
    FvpHeaderView h;
    h.frameIndex = readU32Le(f + 0);
    h.shardIndex = readU16Le(f + 4);
    h.totalShards = readU16Le(f + 6);
    h.flags = readU16Le(f + 8);
    h.dataShards = readU16Le(f + 10);
    if (h.totalShards == 0 || h.totalShards > MAX_FRAME_SHARDS) return false;
    if (h.shardIndex >= h.totalShards) return false;
    if (h.dataShards == 0 || h.dataShards > h.totalShards) return false;
    out = h;
    return true;
}

// RTP sequence number of a received video packet (bytes 2..4, big-endian).
// The engine numbers every packet of a session consecutively, so a gap is
// packet loss. Callers check the length with parseFvpHeader first.
inline uint16_t rtpSequence(const uint8_t* packet) {
    return static_cast<uint16_t>((packet[2] << 8) | packet[3]);
}

// HAPTIC_EVENT payload — Rust engine::HapticEvent::to_payload():
// [controller_id u8][duration_ms u16][frequency f32][amplitude f32], LE.
struct HapticEvent {
    uint8_t controllerId = 0;
    uint16_t durationMs = 0;
    float frequency = 0.0f;
    float amplitude = 0.0f;
};

inline float readF32Le(const uint8_t* p) {
    const uint32_t bits = readU32Le(p);
    float f;
    std::memcpy(&f, &bits, sizeof(f));
    return f;
}

inline bool parseHapticEvent(const uint8_t* payload, size_t len, HapticEvent& out) {
    if (payload == nullptr || len < 11) return false;
    out.controllerId = payload[0];
    out.durationMs = readU16Le(payload + 1);
    out.frequency = readF32Le(payload + 3);
    out.amplitude = readF32Le(payload + 7);
    return true;
}

// HEARTBEAT_ACK payload: the PC's encode and total latency in µs (u32 LE
// each), shown in the HMD's latency overlay.
struct HeartbeatAck {
    uint32_t pcEncodeUs = 0;
    uint32_t pcTotalUs = 0;
};

inline bool parseHeartbeatAck(const uint8_t* payload, size_t len, HeartbeatAck& out) {
    if (payload == nullptr || len < 8) return false;
    out.pcEncodeUs = readU32Le(payload);
    out.pcTotalUs = readU32Le(payload + 4);
    return true;
}

// Resolution to (re)initialise the video decoder with.
struct DecoderDims {
    uint32_t width = 0;
    uint32_t height = 0;
};

// Pick the decoder init resolution: the encoded (actually-decoded) dimensions
// when known, else the native render resolution. Encoded is 0 before
// STREAM_CONFIG arrives or when a legacy server sends no encoded dims — in both
// cases decode at native. Both are per eye; a side-by-side frame holds two.
inline DecoderDims decoderInitDims(uint32_t nativeW, uint32_t nativeH,
                                   uint32_t encodedW, uint32_t encodedH,
                                   uint8_t layout = STEREO_MONO) {
    DecoderDims eye = encodedW > 0 && encodedH > 0 ? DecoderDims{encodedW, encodedH}
                                                   : DecoderDims{nativeW, nativeH};
    if (layout == STEREO_SIDE_BY_SIDE) eye.width *= 2;
    return eye;
}

}  // namespace fvp_client_protocol
