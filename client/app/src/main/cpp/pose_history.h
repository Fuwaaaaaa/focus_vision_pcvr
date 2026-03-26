#pragma once

#include <openxr/openxr.h>
#include <cstdint>
#include <optional>
#include <array>

/// Records the pose at which each video frame was rendered on the PC side.
/// Used by Timewarp to compute the rotation delta for reprojection.
class PoseHistory {
public:
    static constexpr size_t CAPACITY = 16;

    struct Record {
        uint32_t frameIndex;
        XrPosef pose;
        int64_t timestampNs;
    };

    /// Record the render pose for a decoded frame.
    void record(uint32_t frameIndex, const XrPosef& pose, int64_t timestampNs) {
        m_buffer[m_writeIdx] = Record{frameIndex, pose, timestampNs};
        m_writeIdx = (m_writeIdx + 1) % CAPACITY;
        if (m_count < CAPACITY) m_count++;
    }

    /// Find the render pose for a given frame index.
    std::optional<Record> findByFrameIndex(uint32_t frameIndex) const {
        for (size_t i = 0; i < m_count; i++) {
            size_t idx = (m_writeIdx + CAPACITY - 1 - i) % CAPACITY;
            if (m_buffer[idx].frameIndex == frameIndex) {
                return m_buffer[idx];
            }
        }
        return std::nullopt;
    }

    /// Get the most recent record.
    std::optional<Record> latest() const {
        if (m_count == 0) return std::nullopt;
        size_t idx = (m_writeIdx + CAPACITY - 1) % CAPACITY;
        return m_buffer[idx];
    }

private:
    std::array<Record, CAPACITY> m_buffer{};
    size_t m_writeIdx = 0;
    size_t m_count = 0;
};
