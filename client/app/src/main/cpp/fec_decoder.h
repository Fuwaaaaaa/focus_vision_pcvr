#pragma once

#include <cstdint>
#include <vector>
#include <optional>
#include <chrono>

/**
 * Reed-Solomon FEC decoder for the HMD side.
 * Collects shards for each frame, attempts RS reconstruction on timeout.
 *
 * Shard layout per frame:
 *   [0..data_count-1] = data shards
 *   [data_count..total-1] = parity shards
 */
class FecFrameDecoder {
public:
    struct DecodedFrame {
        uint32_t frameIndex;
        std::vector<uint8_t> data;
        bool isKeyframe;
    };

    /// Start collecting shards for a new frame.
    void beginFrame(uint32_t frameIndex, uint16_t totalShards, uint16_t dataShards, bool isKeyframe);

    /// Add a received shard.
    void addShard(uint16_t shardIndex, const uint8_t* data, int dataLen);

    /// Attempt to reconstruct the frame.
    /// Returns the decoded frame data if enough shards are available.
    std::optional<DecodedFrame> tryDecode();

    /// Check if enough shards have been received (data count reached).
    bool isComplete() const;

    uint32_t currentFrameIndex() const { return m_frameIndex; }

private:
    uint32_t m_frameIndex = 0;
    uint16_t m_totalShards = 0;
    uint16_t m_dataShards = 0;
    bool m_isKeyframe = false;
    int m_shardSize = 0;
    uint16_t m_receivedCount = 0;

    // Shard storage: index → data (empty = not received)
    std::vector<std::vector<uint8_t>> m_shards;
    std::vector<bool> m_received;
};

// --- fvp_flags bit layout (matches Rust protocol.rs) ---
namespace fvp_flags {
    inline bool isKeyframe(uint16_t flags) { return (flags & 0x01) != 0; }
    inline uint8_t sliceIndex(uint16_t flags) { return (uint8_t)((flags >> 1) & 0x0F); }
    inline uint8_t sliceCount(uint16_t flags) { return (uint8_t)((flags >> 5) & 0x0F); }
    inline uint8_t streamId(uint16_t flags) { return (uint8_t)((flags >> 9) & 0x03); }
}

/**
 * Slice-based FEC decoder: routes packets to per-slice FecFrameDecoder contexts.
 *
 * When slice_count > 0 in fvp_flags:
 *   - Each slice has an independent RS context (FecFrameDecoder)
 *   - slice_index in fvp_flags determines which context receives the shard
 *   - Completion bitmask tracks which slices have been reconstructed
 *   - Once all slices complete, strips u32 length prefix and concatenates
 *   - 100ms timeout: discard incomplete frame, request IDR
 *
 * When slice_count == 0: falls through to legacy single-context FecFrameDecoder.
 */
class SlicedFecFrameDecoder {
public:
    static constexpr int MAX_SLICES = 15;
    static constexpr auto SLICE_TIMEOUT = std::chrono::milliseconds(100);

    struct DecodedFrame {
        uint32_t frameIndex;
        std::vector<uint8_t> data;
        bool isKeyframe;
    };

    /// Start collecting shards for a new sliced frame.
    /// sliceCount must be > 0 (caller checks fvp_flags).
    void beginFrame(uint32_t frameIndex, uint8_t sliceCount, bool isKeyframe);

    /// Add a shard to the appropriate slice context.
    void addShard(uint8_t sliceIndex, uint16_t shardIndex, uint16_t totalShards,
                  uint16_t dataShards, const uint8_t* data, int dataLen);

    /// Try to assemble the complete frame from all slice contexts.
    /// Returns decoded frame if ALL slices are reconstructed.
    std::optional<DecodedFrame> tryDecode();

    /// Check if all slices have been reconstructed.
    bool isComplete() const;

    /// Check if the frame has timed out (100ms since first shard).
    bool isTimedOut() const;

    uint32_t currentFrameIndex() const { return m_frameIndex; }
    uint8_t sliceCount() const { return m_sliceCount; }

private:
    uint32_t m_frameIndex = 0;
    uint8_t m_sliceCount = 0;
    bool m_isKeyframe = false;
    uint16_t m_sliceCompleted = 0; // bitmask: bit i = slice i decoded
    std::chrono::steady_clock::time_point m_startTime;
    bool m_started = false;

    // Per-slice RS contexts
    FecFrameDecoder m_contexts[MAX_SLICES];
    // Per-slice decoded data (cached after successful tryDecode on each context)
    std::vector<uint8_t> m_sliceData[MAX_SLICES];
};
