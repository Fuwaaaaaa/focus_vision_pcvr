#include "fec_decoder.h"
#include "xr_utils.h"

void FecFrameDecoder::beginFrame(uint32_t frameIndex, uint8_t totalShards,
                                  uint8_t dataShards, bool isKeyframe) {
    m_frameIndex = frameIndex;
    m_totalShards = totalShards;
    m_dataShards = dataShards;
    m_isKeyframe = isKeyframe;
    m_shardSize = 0;
    m_receivedCount = 0;
    m_shards.assign(totalShards, std::vector<uint8_t>());
    m_received.assign(totalShards, false);
}

void FecFrameDecoder::addShard(uint8_t shardIndex, const uint8_t* data, int dataLen) {
    if (shardIndex >= m_totalShards) return;
    if (m_received[shardIndex]) return; // duplicate

    m_shards[shardIndex].assign(data, data + dataLen);
    m_received[shardIndex] = true;
    m_receivedCount++;

    if (m_shardSize == 0) {
        m_shardSize = dataLen;
    }
}

bool FecFrameDecoder::isComplete() const {
    // We need at least data_shard_count shards (any combination of data + parity)
    return m_receivedCount >= m_dataShards;
}

std::optional<FecFrameDecoder::DecodedFrame> FecFrameDecoder::tryDecode() {
    if (!isComplete()) {
        return std::nullopt;
    }

    // Simple path: if all data shards are received, just concatenate them
    bool allDataPresent = true;
    for (uint8_t i = 0; i < m_dataShards; i++) {
        if (!m_received[i]) {
            allDataPresent = false;
            break;
        }
    }

    if (allDataPresent) {
        DecodedFrame frame;
        frame.frameIndex = m_frameIndex;
        frame.isKeyframe = m_isKeyframe;
        for (uint8_t i = 0; i < m_dataShards; i++) {
            frame.data.insert(frame.data.end(), m_shards[i].begin(), m_shards[i].end());
        }
        return frame;
    }

    // FEC reconstruction needed — would use a C Reed-Solomon library here.
    // For v1.0: log the loss and skip the frame (timewarp will cover).
    // Full RS decode will be added when a C-compatible RS library is integrated.
    uint8_t missing = 0;
    for (uint8_t i = 0; i < m_dataShards; i++) {
        if (!m_received[i]) missing++;
    }
    LOGW("FEC: frame %u needs reconstruction (%u data shards missing), "
         "received %u/%u total shards",
         m_frameIndex, missing, m_receivedCount, m_totalShards);

    // Attempt partial assembly (skip missing shards, fill with zeros)
    // This produces a corrupted frame, but is better than nothing with timewarp
    DecodedFrame frame;
    frame.frameIndex = m_frameIndex;
    frame.isKeyframe = m_isKeyframe;
    for (uint8_t i = 0; i < m_dataShards; i++) {
        if (m_received[i]) {
            frame.data.insert(frame.data.end(), m_shards[i].begin(), m_shards[i].end());
        } else {
            // Fill missing shard with zeros (will cause decode artifact)
            frame.data.insert(frame.data.end(), m_shardSize, 0);
        }
    }
    return frame;
}
