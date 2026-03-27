#include "nal_validator.h"

// H.265 NAL unit types (ITU-T H.265 Table 7-1)
enum HevcNalType {
    HEVC_NAL_TRAIL_N = 0,
    HEVC_NAL_TRAIL_R = 1,
    HEVC_NAL_BLA_W_LP = 16,
    HEVC_NAL_IDR_W_RADL = 19,
    HEVC_NAL_IDR_N_LP = 20,
    HEVC_NAL_CRA_NUT = 21,
    HEVC_NAL_VPS = 32,
    HEVC_NAL_SPS = 33,
    HEVC_NAL_PPS = 34,
    HEVC_NAL_AUD = 35,
    HEVC_NAL_UNSPEC63 = 63,
};

NalValidator::Result NalValidator::validate(const uint8_t* data, int size) {
    // Minimum NAL unit: 2 bytes header + at least 1 byte payload
    if (!data || size < 3) {
        return Result::TooShort;
    }

    // Byte 0: forbidden_zero_bit(1) | nal_unit_type(6) | nuh_layer_id_high(1)
    uint8_t byte0 = data[0];
    uint8_t forbiddenBit = (byte0 >> 7) & 0x01;
    if (forbiddenBit != 0) {
        return Result::ForbiddenBit;
    }

    uint8_t nalType = (byte0 >> 1) & 0x3F;
    if (nalType > HEVC_NAL_UNSPEC63) {
        return Result::InvalidType;
    }

    // Byte 1: nuh_layer_id_low(5) | nuh_temporal_id_plus1(3)
    uint8_t byte1 = data[1];
    uint8_t tidPlus1 = byte1 & 0x07;
    if (tidPlus1 == 0) {
        return Result::InvalidTid;
    }

    return Result::Valid;
}

uint8_t NalValidator::getNalType(const uint8_t* data) {
    return (data[0] >> 1) & 0x3F;
}

bool NalValidator::isIdr(uint8_t nalType) {
    return nalType == HEVC_NAL_IDR_W_RADL || nalType == HEVC_NAL_IDR_N_LP;
}

bool NalValidator::isParameterSet(uint8_t nalType) {
    return nalType == HEVC_NAL_VPS || nalType == HEVC_NAL_SPS || nalType == HEVC_NAL_PPS;
}
