#pragma once

#include <cstdint>
#include <vector>
#include <optional>

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
    void beginFrame(uint32_t frameIndex, uint8_t totalShards, uint8_t dataShards, bool isKeyframe);

    /// Add a received shard.
    void addShard(uint8_t shardIndex, const uint8_t* data, int dataLen);

    /// Attempt to reconstruct the frame.
    /// Returns the decoded frame data if enough shards are available.
    std::optional<DecodedFrame> tryDecode();

    /// Check if enough shards have been received (data count reached).
    bool isComplete() const;

    uint32_t currentFrameIndex() const { return m_frameIndex; }

private:
    uint32_t m_frameIndex = 0;
    uint8_t m_totalShards = 0;
    uint8_t m_dataShards = 0;
    bool m_isKeyframe = false;
    int m_shardSize = 0;
    uint8_t m_receivedCount = 0;

    // Shard storage: index → data (empty = not received)
    std::vector<std::vector<uint8_t>> m_shards;
    std::vector<bool> m_received;
};
