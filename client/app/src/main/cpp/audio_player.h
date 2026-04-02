#pragma once

#include <cstdint>
#include <vector>
#include <atomic>
#include <mutex>

// Forward declarations (Android NDK headers)
struct OpusDecoder;
struct AAudioStream;

/**
 * Audio receiver and player for the HMD side.
 *
 * Receives Opus-encoded audio via UDP, decodes to PCM,
 * and plays through Android's AAudio API (low-latency mode).
 *
 * Flow:
 *   UDP Opus packet → submitOpusPacket() → opus_decode → jitter buffer
 *   pump() → AAudioStream_write → HMD speakers
 */
class AudioPlayer {
public:
    bool init(int sampleRate = 48000, int channels = 2);
    void shutdown();

    /// Submit an Opus packet for decode and playback.
    /// Called from the network receive thread. Thread-safe.
    bool submitOpusPacket(const uint8_t* data, int size);

    /// Feed decoded samples from jitter buffer to AAudio output.
    /// Call from the main loop or a dedicated audio thread.
    void pump();

    bool isInitialized() const { return m_initialized; }

private:
    std::atomic<bool> m_initialized{false};
    int m_sampleRate = 48000;
    int m_channels = 2;

    // Opus decoder
    OpusDecoder* m_opusDecoder = nullptr;

    // AAudio output stream
    AAudioStream* m_stream = nullptr;

    // Jitter buffer: decoded PCM samples waiting to be written to AAudio.
    // Mutex protects access from network thread (submitOpusPacket) and
    // audio thread (pump). Contention is low: decode is fast (<1ms).
    std::mutex m_bufferMutex;
    std::vector<int16_t> m_pcmBuffer;

    // Decode workspace (avoids per-packet allocation)
    std::vector<int16_t> m_decodeBuffer;

    static constexpr int FRAME_SIZE = 480; // 10ms at 48kHz
    static constexpr int MAX_BUFFER_MS = 100; // Max jitter buffer depth
};
