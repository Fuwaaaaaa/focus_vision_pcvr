#pragma once

#include <cstdint>
#include <vector>
#include <atomic>

/**
 * Audio receiver and player for the HMD side.
 *
 * Receives Opus-encoded audio via UDP, decodes to PCM,
 * and plays through Android's AudioTrack API (low-latency mode).
 *
 * Includes a simple jitter buffer to absorb packet arrival variance.
 */
class AudioPlayer {
public:
    bool init(int sampleRate = 48000, int channels = 2);
    void shutdown();

    /// Submit an Opus packet for decode and playback.
    /// Called from the network receive thread.
    bool submitOpusPacket(const uint8_t* data, int size);

    /// Call periodically to feed decoded samples to AudioTrack.
    void pump();

    bool isInitialized() const { return m_initialized; }

private:
    std::atomic<bool> m_initialized{false};
    int m_sampleRate = 48000;
    int m_channels = 2;

    // Opus decoder handle (opaque, from libopus)
    void* m_opusDecoder = nullptr;

    // Decoded PCM buffer (jitter buffer)
    std::vector<int16_t> m_pcmBuffer;
    static constexpr int FRAME_SIZE = 480; // 10ms at 48kHz
};
