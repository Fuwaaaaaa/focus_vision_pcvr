#pragma once

#include <cstdint>
#include <cmath>
#include <vector>

/// Foveated encoding preset: QP offset values for mid and peripheral zones.
struct FoveatedPreset {
    const char* name;
    int8_t mid_qp_offset;
    int8_t peripheral_qp_offset;
};

static constexpr FoveatedPreset FOVEATED_PRESETS[] = {
    {"subtle",     3,  8},
    {"balanced",   5, 15},
    {"aggressive", 8, 25},
};
static constexpr uint32_t FOVEATED_PRESET_COUNT = 3;

/// Find a foveated preset by name. Returns nullptr if not found.
inline const FoveatedPreset* findFoveatedPreset(const char* name) {
    for (uint32_t i = 0; i < FOVEATED_PRESET_COUNT; ++i) {
        // Simple string compare (no strcmp dependency in header-only)
        const char* a = FOVEATED_PRESETS[i].name;
        const char* b = name;
        bool match = true;
        while (*a && *b) {
            if (*a++ != *b++) { match = false; break; }
        }
        if (match && *a == '\0' && *b == '\0') return &FOVEATED_PRESETS[i];
    }
    return nullptr;
}

/// Compute CTU grid dimensions for a given frame size and CTU block size.
inline void computeCtuGrid(uint32_t width, uint32_t height, uint32_t ctuSize,
                           uint32_t& outCols, uint32_t& outRows) {
    outCols = (width + ctuSize - 1) / ctuSize;
    outRows = (height + ctuSize - 1) / ctuSize;
}

/// Compute QP delta map for foveated encoding.
/// Pure function: no NVENC or GPU dependencies.
///
/// @param gazeX Normalized gaze X (0.0 = left, 1.0 = right)
/// @param gazeY Normalized gaze Y (0.0 = top, 1.0 = bottom)
/// @param ctuCols Number of CTU columns
/// @param ctuRows Number of CTU rows
/// @param foveaRadius Fovea radius as fraction of frame width (e.g. 0.15)
/// @param midRadius Mid-zone radius as fraction of frame width (e.g. 0.35)
/// @param midQpDelta QP offset for mid zone
/// @param peripheralQpDelta QP offset for peripheral zone
/// @param outMap Output QP delta map (resized to ctuCols * ctuRows)
inline void computeQpDeltaMap(
    float gazeX, float gazeY,
    uint32_t ctuCols, uint32_t ctuRows,
    float foveaRadius, float midRadius,
    int8_t midQpDelta, int8_t peripheralQpDelta,
    std::vector<int8_t>& outMap)
{
    const uint32_t mapSize = ctuCols * ctuRows;
    outMap.resize(mapSize);

    const float gazeCtuX = gazeX * static_cast<float>(ctuCols);
    const float gazeCtuY = gazeY * static_cast<float>(ctuRows);
    const float foveaCtu = foveaRadius * static_cast<float>(ctuCols);
    const float midCtu = midRadius * static_cast<float>(ctuCols);

    for (uint32_t row = 0; row < ctuRows; ++row) {
        for (uint32_t col = 0; col < ctuCols; ++col) {
            float dx = static_cast<float>(col) + 0.5f - gazeCtuX;
            float dy = static_cast<float>(row) + 0.5f - gazeCtuY;
            float dist = sqrtf(dx * dx + dy * dy);

            int8_t delta;
            if (dist <= foveaCtu) {
                delta = 0;
            } else if (dist <= midCtu) {
                delta = midQpDelta;
            } else {
                delta = peripheralQpDelta;
            }
            outMap[row * ctuCols + col] = delta;
        }
    }
}
