#pragma once

#include <cstdint>
#include <cmath>

// Hardware-independent encode-parameter math. The encoded frame dimensions are
// NOT recomputed here — the driver reads them from FvpConfig.encoded_* (the
// engine is the single source of truth, so PC encode resolution always matches
// the STREAM_CONFIG dims sent to the client). Only the bitrate derivation lives
// driver-side because it is an NVENC rate-control parameter.
namespace fvp_encode {

/// NVENC average/target bitrate in bits per second for an encoded frame size:
/// bitrate = encoded_w * encoded_h * pixel_factor. The default factor (2.0)
/// reproduces the historical `width * height * 2` formula; exposing it as a
/// parameter lets quality at sub-native resolution be tuned without a rebuild.
inline uint32_t computeBitrateBps(uint32_t encodedW, uint32_t encodedH, float pixelFactor) {
    const double bits = static_cast<double>(encodedW)
                      * static_cast<double>(encodedH)
                      * static_cast<double>(pixelFactor);
    if (bits <= 0.0) {
        return 0;
    }
    const double maxU32 = static_cast<double>(UINT32_MAX);
    const double clamped = bits > maxU32 ? maxU32 : bits;
    return static_cast<uint32_t>(std::llround(clamped));
}

}  // namespace fvp_encode
