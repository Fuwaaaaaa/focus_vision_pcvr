#pragma once

#include <cstdint>

// Hardware-independent encode-parameter helpers. The engine is the single
// source of truth for encode parameters: the driver reads the encoded frame
// dimensions (FvpConfig.encoded_*) and the target bitrate
// (FvpConfig.bitrate_bps, from `[video] bitrate_mbps`) instead of deriving
// them, so NVENC, STREAM_CONFIG and the adaptive bitrate controller agree.
namespace fvp_encode {

/// Bitrate used when the engine config is unavailable. Matches the default
/// `[video] bitrate_mbps = 80`.
constexpr uint32_t kDefaultBitrateBps = 80'000'000;

/// NVENC average/target bitrate in bits per second from the engine config.
/// 0 means the engine did not provide one (older engine, unset struct).
inline uint32_t targetBitrateBps(uint32_t configuredBps) {
    return configuredBps != 0 ? configuredBps : kDefaultBitrateBps;
}

}  // namespace fvp_encode
