#include "audio_player.h"
#include "xr_utils.h"

// AudioPlayer is a stub that defines the interface.
// Full implementation requires:
//   - libopus for decoding (NDK CMake dependency)
//   - Android AAudio or AudioTrack for playback
//
// For v0.2, the structure is ready. The actual Opus decode + AudioTrack
// playback will be connected when the Android NDK build is set up with
// libopus as a dependency.

bool AudioPlayer::init(int sampleRate, int channels) {
    m_sampleRate = sampleRate;
    m_channels = channels;

    // Reserve buffer for 100ms of audio (jitter buffer)
    m_pcmBuffer.reserve(sampleRate / 10 * channels);

    // TODO: Initialize Opus decoder
    //   m_opusDecoder = opus_decoder_create(sampleRate, channels, &error);

    // TODO: Initialize AAudio stream
    //   AAudioStreamBuilder_create(&builder);
    //   AAudioStreamBuilder_setSampleRate(builder, sampleRate);
    //   AAudioStreamBuilder_setChannelCount(builder, channels);
    //   AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_I16);
    //   AAudioStreamBuilder_setPerformanceMode(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);

    m_initialized = true;
    LOGI("AudioPlayer initialized: %dHz, %d ch", sampleRate, channels);
    return true;
}

void AudioPlayer::shutdown() {
    // TODO: opus_decoder_destroy(m_opusDecoder);
    // TODO: AAudioStream_close(stream);
    m_opusDecoder = nullptr;
    m_pcmBuffer.clear();
    m_initialized = false;
    LOGI("AudioPlayer shut down");
}

bool AudioPlayer::submitOpusPacket(const uint8_t* data, int size) {
    if (!m_initialized) return false;

    // TODO: Decode Opus packet to PCM
    //   int samples = opus_decode(m_opusDecoder, data, size,
    //                             decodeBuffer, FRAME_SIZE, 0);
    //   Append decoded samples to m_pcmBuffer (jitter buffer)

    // For now, just acknowledge receipt
    return true;
}

void AudioPlayer::pump() {
    if (!m_initialized) return;

    // TODO: Write decoded PCM from jitter buffer to AAudio stream
    //   AAudioStream_write(stream, m_pcmBuffer.data(), framesAvailable, 0);
    //   Remove written samples from buffer
}
