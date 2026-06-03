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
// v3 wire format (FVP slice/stream flags — see fec_decoder.h fvp_flags).
inline constexpr uint16_t PROTOCOL_VERSION = 3;

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
