#pragma once

#include <cstdint>

/**
 * H.265 (HEVC) NAL unit validator.
 *
 * Validates NAL unit headers before passing to MediaCodec to prevent
 * decoder crashes from corrupted data (eng review #3).
 *
 * H.265 NAL header (2 bytes):
 *   forbidden_zero_bit (1 bit) = 0
 *   nal_unit_type (6 bits)
 *   nuh_layer_id (6 bits)
 *   nuh_temporal_id_plus1 (3 bits) >= 1
 */
class NalValidator {
public:
    enum class Result {
        Valid,
        TooShort,       // NAL unit shorter than minimum header
        ForbiddenBit,   // forbidden_zero_bit is set (corrupted)
        InvalidType,    // nal_unit_type out of valid range
        InvalidTid,     // nuh_temporal_id_plus1 is 0
    };

    /// Validate an H.265 NAL unit (after start code removal).
    /// `data` points to the first byte of the NAL header.
    /// `size` is the total NAL unit size including header.
    static Result validate(const uint8_t* data, int size);

    /// Extract the NAL unit type from a valid NAL header.
    static uint8_t getNalType(const uint8_t* data);

    /// Check if a NAL type is an IDR picture.
    static bool isIdr(uint8_t nalType);

    /// Check if a NAL type is a parameter set (VPS/SPS/PPS).
    static bool isParameterSet(uint8_t nalType);
};
