#include "fec_decoder.h"
#include "xr_utils.h"
#include <mutex>

void FecFrameDecoder::beginFrame(uint32_t frameIndex, uint16_t totalShards,
                                  uint16_t dataShards, bool isKeyframe) {
    m_frameIndex = frameIndex;
    m_totalShards = totalShards;
    m_dataShards = dataShards;
    m_isKeyframe = isKeyframe;
    m_shardSize = 0;
    m_receivedCount = 0;
    m_shards.assign(totalShards, std::vector<uint8_t>());
    m_received.assign(totalShards, false);
}

void FecFrameDecoder::addShard(uint16_t shardIndex, const uint8_t* data, int dataLen) {
    if (dataLen <= 0) return;
    if (shardIndex >= m_totalShards) return;
    if (m_received[shardIndex]) return; // duplicate

    if (m_shardSize != 0 && dataLen != m_shardSize) return; // mismatched shard size

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
    for (uint16_t i = 0; i < m_dataShards; i++) {
        if (!m_received[i]) {
            allDataPresent = false;
            break;
        }
    }

    if (allDataPresent) {
        DecodedFrame frame;
        frame.frameIndex = m_frameIndex;
        frame.isKeyframe = m_isKeyframe;
        for (uint16_t i = 0; i < m_dataShards; i++) {
            frame.data.insert(frame.data.end(), m_shards[i].begin(), m_shards[i].end());
        }
        return frame;
    }

    // FEC reconstruction: use Reed-Solomon to recover missing data shards
    // from parity shards. We need at least m_dataShards total shards
    // (any combination of data + parity) to reconstruct.
    uint16_t missing = 0;
    for (uint16_t i = 0; i < m_dataShards; i++) {
        if (!m_received[i]) missing++;
    }

    uint16_t availableParity = 0;
    for (uint16_t i = m_dataShards; i < m_totalShards; i++) {
        if (m_received[i]) availableParity++;
    }

    if (m_shardSize == 0) {
        LOGW("FEC: frame %u has zero shard size, cannot reconstruct", m_frameIndex);
        return std::nullopt;
    }

    if (missing > availableParity) {
        // Not enough parity shards to reconstruct — too much loss
        LOGW("FEC: frame %u unrecoverable (%u data missing, %u parity available)",
             m_frameIndex, missing, availableParity);
        return std::nullopt;
    }

    // Reed-Solomon reconstruction via Vandermonde matrix inversion.
    // We have m_dataShards unknowns and m_dataShards equations from
    // the received data + parity shards.
    //
    // For each missing data shard, we substitute a parity shard equation.
    // The RS code used on the Rust side (reed-solomon-erasure crate) uses
    // a standard Vandermonde/Cauchy matrix over GF(2^8).
    //
    // This implementation uses the same GF(2^8) arithmetic with the
    // primitive polynomial 0x11d (x^8 + x^4 + x^3 + x^2 + 1),
    // matching reed-solomon-erasure's default.

    // Build GF(2^8) tables (thread-safe initialization)
    static uint8_t gfExp[512];
    static uint8_t gfLog[256];
    static std::once_flag gfOnce;
    std::call_once(gfOnce, []() {
        // Primitive polynomial: x^8 + x^4 + x^3 + x^2 + 1 = 0x11d
        uint16_t x = 1;
        for (int i = 0; i < 255; i++) {
            gfExp[i] = (uint8_t)x;
            gfLog[x] = (uint8_t)i;
            x <<= 1;
            if (x & 0x100) x ^= 0x11d;
        }
        for (int i = 255; i < 512; i++) gfExp[i] = gfExp[i - 255];
        gfLog[0] = 0; // undefined, but avoid garbage
    });

    auto gfMul = [&](uint8_t a, uint8_t b) -> uint8_t {
        if (a == 0 || b == 0) return 0;
        return gfExp[gfLog[a] + gfLog[b]];
    };
    auto gfInv = [&](uint8_t a) -> uint8_t {
        if (a == 0) return 0; // shouldn't happen
        return gfExp[255 - gfLog[a]];
    };

    // gf_pow: compute a^n in GF(2^8)
    auto gfPow = [&](uint8_t a, uint16_t n) -> uint8_t {
        if (n == 0) return 1;
        if (a == 0) return 0;
        uint16_t logA = gfLog[a];
        uint16_t logResult = (uint16_t)((uint32_t)logA * n % 255);
        return gfExp[logResult];
    };

    // Collect indices of received shards (data + parity, up to m_dataShards count)
    std::vector<uint16_t> presentIdx;
    for (uint16_t i = 0; i < m_totalShards && presentIdx.size() < m_dataShards; i++) {
        if (m_received[i]) presentIdx.push_back(i);
    }

    // Build the encoding matrix matching reed-solomon-erasure crate:
    // 1. Vandermonde matrix V where V[i][j] = i^j in GF(2^8) (i = row index as field element)
    // 2. Encoding matrix E = V × V_top^(-1), giving identity for data rows
    // 3. For decoding: select rows of E for received shards, invert, multiply
    uint16_t n = m_dataShards;
    uint16_t total = m_totalShards;

    // Step 1: Build full Vandermonde matrix (total × n)
    std::vector<std::vector<uint8_t>> vand(total, std::vector<uint8_t>(n, 0));
    for (uint16_t i = 0; i < total; i++) {
        for (uint16_t j = 0; j < n; j++) {
            vand[i][j] = gfPow((uint8_t)(i & 0xFF), j);
        }
    }

    // Step 2: Compute V_top^(-1) via Gaussian elimination on top n×n
    std::vector<std::vector<uint8_t>> topInv(n, std::vector<uint8_t>(2 * n, 0));
    for (uint16_t r = 0; r < n; r++) {
        for (uint16_t c = 0; c < n; c++) topInv[r][c] = vand[r][c];
        topInv[r][n + r] = 1;
    }
    for (uint16_t col = 0; col < n; col++) {
        uint16_t pivot = col;
        while (pivot < n && topInv[pivot][col] == 0) pivot++;
        if (pivot == n) {
            LOGW("FEC: V_top singular at col %u", col);
            return std::nullopt;
        }
        if (pivot != col) std::swap(topInv[pivot], topInv[col]);
        uint8_t inv = gfInv(topInv[col][col]);
        for (uint16_t c = 0; c < 2 * n; c++) topInv[col][c] = gfMul(topInv[col][c], inv);
        for (uint16_t r = 0; r < n; r++) {
            if (r == col || topInv[r][col] == 0) continue;
            uint8_t factor = topInv[r][col];
            for (uint16_t c = 0; c < 2 * n; c++) topInv[r][c] ^= gfMul(factor, topInv[col][c]);
        }
    }

    // Step 3: Build encoding matrix E = V × V_top^(-1)
    // We only need the rows corresponding to received shards
    std::vector<std::vector<uint8_t>> matrix(n, std::vector<uint8_t>(n, 0));
    for (uint16_t row = 0; row < n; row++) {
        uint16_t s = presentIdx[row];
        for (uint16_t col = 0; col < n; col++) {
            uint8_t val = 0;
            for (uint16_t k = 0; k < n; k++) {
                val ^= gfMul(vand[s][k], topInv[k][n + col]);
            }
            matrix[row][col] = val;
        }
    }

    // Gaussian elimination to invert the matrix
    // Augment with identity
    std::vector<std::vector<uint8_t>> aug(n, std::vector<uint8_t>(2 * n, 0));
    for (uint16_t r = 0; r < n; r++) {
        for (uint16_t c = 0; c < n; c++) aug[r][c] = matrix[r][c];
        aug[r][n + r] = 1;
    }

    for (uint16_t col = 0; col < n; col++) {
        // Find pivot
        uint16_t pivot = col;
        while (pivot < n && aug[pivot][col] == 0) pivot++;
        if (pivot == n) {
            LOGW("FEC: frame %u RS matrix singular at col %u", m_frameIndex, col);
            return std::nullopt;
        }
        if (pivot != col) std::swap(aug[pivot], aug[col]);

        // Scale pivot row
        uint8_t inv = gfInv(aug[col][col]);
        for (uint16_t c = 0; c < 2 * n; c++) aug[col][c] = gfMul(aug[col][c], inv);

        // Eliminate column
        for (uint16_t r = 0; r < n; r++) {
            if (r == col || aug[r][col] == 0) continue;
            uint8_t factor = aug[r][col];
            for (uint16_t c = 0; c < 2 * n; c++) {
                aug[r][c] ^= gfMul(factor, aug[col][c]);
            }
        }
    }

    // Reconstruct: multiply inverse matrix by received shard data
    DecodedFrame frame;
    frame.frameIndex = m_frameIndex;
    frame.isKeyframe = m_isKeyframe;
    size_t totalSize = (size_t)m_dataShards * m_shardSize;
    if (m_shardSize > 0 && totalSize / m_shardSize != m_dataShards) {
        LOGE("FEC: integer overflow in frame size (%u * %d)", m_dataShards, m_shardSize);
        return {};
    }
    frame.data.resize(totalSize, 0);

    for (uint16_t di = 0; di < m_dataShards; di++) {
        for (int byteIdx = 0; byteIdx < m_shardSize; byteIdx++) {
            uint8_t val = 0;
            for (uint16_t k = 0; k < n; k++) {
                uint8_t coeff = aug[di][n + k];
                uint8_t srcByte = m_shards[presentIdx[k]].size() > (size_t)byteIdx
                    ? m_shards[presentIdx[k]][byteIdx] : 0;
                val ^= gfMul(coeff, srcByte);
            }
            frame.data[di * m_shardSize + byteIdx] = val;
        }
    }

    LOGI("FEC: frame %u reconstructed (%u data shards recovered from %u parity)",
         m_frameIndex, missing, availableParity);
    return frame;
}

// --- SlicedFecFrameDecoder ---

void SlicedFecFrameDecoder::beginFrame(uint32_t frameIndex, uint8_t sliceCount, bool isKeyframe) {
    m_frameIndex = frameIndex;
    m_sliceCount = (sliceCount > MAX_SLICES) ? MAX_SLICES : sliceCount;
    m_isKeyframe = isKeyframe;
    m_sliceCompleted = 0;
    m_started = false;
    for (int i = 0; i < m_sliceCount; i++) {
        m_sliceData[i].clear();
    }
}

void SlicedFecFrameDecoder::addShard(uint8_t sliceIndex, uint16_t shardIndex,
                                      uint16_t totalShards, uint16_t dataShards,
                                      const uint8_t* data, int dataLen) {
    if (sliceIndex >= m_sliceCount) return;

    if (!m_started) {
        m_startTime = std::chrono::steady_clock::now();
        m_started = true;
    }

    // Initialize the per-slice context if this is the first shard for this slice
    auto& ctx = m_contexts[sliceIndex];
    if (ctx.currentFrameIndex() != m_frameIndex) {
        ctx.beginFrame(m_frameIndex, totalShards, dataShards, m_isKeyframe);
    }

    ctx.addShard(shardIndex, data, dataLen);

    // If this slice is now complete and not yet decoded, try to decode it
    if (!(m_sliceCompleted & (1 << sliceIndex)) && ctx.isComplete()) {
        auto decoded = ctx.tryDecode();
        if (decoded.has_value()) {
            // Strip u32 length prefix from decoded data.
            //
            // SECURITY: originalLen is attacker-influenced (UDP payload post-FEC).
            // The earlier `originalLen + 4 <= data.size()` check was computed in
            // uint32_t — with originalLen near UINT32_MAX the addition wrapped
            // and the subsequent `data.begin() + 4 + originalLen` iterator was
            // advanced past end, producing heap OOB read. We promote to size_t
            // and cap at MAX_DECODED_SLICE_BYTES (16 MiB) so a malformed length
            // prefix can never drive an out-of-bounds assign. On failure we
            // leave the slice empty and let the existing SLICE_TIMEOUT path
            // request an IDR (already rate-limited to 2/sec).
            static constexpr size_t MAX_DECODED_SLICE_BYTES = 16 * 1024 * 1024;
            if (decoded->data.size() >= 4) {
                size_t originalLen =
                      static_cast<size_t>(decoded->data[0])
                    | (static_cast<size_t>(decoded->data[1]) << 8)
                    | (static_cast<size_t>(decoded->data[2]) << 16)
                    | (static_cast<size_t>(decoded->data[3]) << 24);

                const size_t available = decoded->data.size() - 4;
                if (originalLen > MAX_DECODED_SLICE_BYTES || originalLen > available) {
                    LOGW("SlicedFecFrameDecoder: invalid length prefix %zu "
                         "(available=%zu, cap=%zu) for slice %u — dropping",
                         originalLen, available, MAX_DECODED_SLICE_BYTES,
                         (unsigned)sliceIndex);
                    return;
                }
                m_sliceData[sliceIndex].assign(
                    decoded->data.begin() + 4,
                    decoded->data.begin() + 4 + static_cast<ptrdiff_t>(originalLen)
                );
            } else {
                m_sliceData[sliceIndex] = std::move(decoded->data);
            }
            m_sliceCompleted |= (1 << sliceIndex);
        }
    }
}

bool SlicedFecFrameDecoder::isComplete() const {
    uint16_t allBits = (1 << m_sliceCount) - 1;
    return (m_sliceCompleted & allBits) == allBits;
}

bool SlicedFecFrameDecoder::isTimedOut() const {
    if (!m_started) return false;
    return (std::chrono::steady_clock::now() - m_startTime) > SLICE_TIMEOUT;
}

std::optional<SlicedFecFrameDecoder::DecodedFrame> SlicedFecFrameDecoder::tryDecode() {
    if (!isComplete()) {
        return std::nullopt;
    }

    DecodedFrame frame;
    frame.frameIndex = m_frameIndex;
    frame.isKeyframe = m_isKeyframe;

    // Concatenate all slice data in order
    size_t totalSize = 0;
    for (int i = 0; i < m_sliceCount; i++) {
        totalSize += m_sliceData[i].size();
    }
    frame.data.reserve(totalSize);
    for (int i = 0; i < m_sliceCount; i++) {
        frame.data.insert(frame.data.end(), m_sliceData[i].begin(), m_sliceData[i].end());
    }

    return frame;
}
