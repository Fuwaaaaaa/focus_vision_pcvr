#pragma once
// Hardware-independent control-protocol helpers for the Android client.
//
// Kept free of Android / mbedtls / OpenXR dependencies so the logic is
// host-buildable and unit-testable (see client/tests/). Wire formats here MUST
// match the Rust side (rust/common/src/protocol.rs).
#include <array>
#include <cstddef>
#include <cstdint>

namespace fvp_client_protocol {

// Protocol version — must match Rust PROTOCOL_VERSION. The client implements the
// v4 wire format: v3 FVP slice/stream flags (see fec_decoder.h fvp_flags) plus
// the 12-byte FVP header carrying data_shard_count (parseFvpHeader below).
inline constexpr uint16_t PROTOCOL_VERSION = 4;

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
// resolution; `encodedWidth`/`encodedHeight` are what is actually decoded —
// equal to native unless the server downscaled (resolution_scale).
struct StreamConfigView {
    uint32_t width = 0;
    uint32_t height = 0;
    uint32_t bitrateMbps = 0;
    uint32_t framerate = 0;
    uint8_t codec = 0;
    uint32_t encodedWidth = 0;
    uint32_t encodedHeight = 0;
};

// Parse a STREAM_CONFIG payload. Layout (little-endian) — see Rust
// encode_stream_config():
//   [0..4] render_w | [4..8] render_h | [8..12] bitrate | [12..16] framerate |
//   [16] codec | [17..21] encoded_w | [21..25] encoded_h
// A payload of >= 25 bytes carries explicit encoded dims; a legacy 17..24-byte
// payload (old server) has none, so encoded falls back to native. Returns false
// for a payload shorter than the 17-byte minimum.
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

// Resolution to (re)initialise the video decoder with.
struct DecoderDims {
    uint32_t width = 0;
    uint32_t height = 0;
};

// Pick the decoder init resolution: the encoded (actually-decoded) dimensions
// when known, else the native render resolution. Encoded is 0 before
// STREAM_CONFIG arrives or when a legacy server sends no encoded dims — in both
// cases decode at native.
inline DecoderDims decoderInitDims(uint32_t nativeW, uint32_t nativeH,
                                   uint32_t encodedW, uint32_t encodedH) {
    if (encodedW > 0 && encodedH > 0) {
        return {encodedW, encodedH};
    }
    return {nativeW, nativeH};
}

}  // namespace fvp_client_protocol
