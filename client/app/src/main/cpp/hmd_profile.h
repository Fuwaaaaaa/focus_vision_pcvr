#pragma once

#include <cstdint>
#include <string>

/**
 * HMD hardware profile — abstracts device-specific parameters.
 *
 * Each supported HMD gets a profile that defines its display, codec,
 * and tracking capabilities. The streaming engine uses this to configure
 * encoding, rendering, and network parameters per-device.
 *
 * Adding a new HMD:
 *   1. Add an entry to HmdType enum
 *   2. Create a profile in HmdProfile::detect() or HmdProfile::forType()
 *   3. The rest of the pipeline adapts automatically
 */

enum class HmdType {
    FocusVision,   // VIVE Focus Vision (XR2 Gen 2)
    Quest3,        // Meta Quest 3 (XR2 Gen 2)
    Quest3S,       // Meta Quest 3S (XR2 Gen 2, lower res)
    QuestPro,      // Meta Quest Pro (XR2+)
    VisionPro,     // Apple Vision Pro (M2)
    PicoNeo4,      // Pico Neo 4 (XR2 Gen 2)
    Generic,       // Unknown HMD — safe defaults
};

struct DisplayProfile {
    uint32_t widthPerEye;
    uint32_t heightPerEye;
    uint32_t refreshRate;        // Hz
    float ipd;                   // meters
    bool hasEyeTracking;
    bool supportsFoveatedDecode; // HW-level foveated decode support
};

struct CodecProfile {
    bool supportsH265;
    bool supportsH264;
    bool supportsAV1;
    uint32_t maxDecodeBitrateKbps;
    uint32_t recommendedBitrateKbps;
};

struct HmdProfile {
    HmdType type;
    std::string name;
    DisplayProfile display;
    CodecProfile codec;

    /// Detect the connected HMD type from OpenXR system properties.
    /// Falls back to Generic if unrecognized.
    static HmdProfile detect(const char* systemName, const char* vendorName);

    /// Get a profile for a known HMD type.
    static HmdProfile forType(HmdType type);
};
