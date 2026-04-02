#include "audio_player.h"
#include "xr_utils.h"

#include <opus/opus.h>
#include <aaudio/AAudio.h>
#include <cstring>

bool AudioPlayer::init(int sampleRate, int channels) {
    if (m_initialized) return true;

    m_sampleRate = sampleRate;
    m_channels = channels;

    // --- Opus decoder ---
    int opusErr = 0;
    m_opusDecoder = opus_decoder_create(sampleRate, channels, &opusErr);
    if (opusErr != OPUS_OK || !m_opusDecoder) {
        LOGE("AudioPlayer: opus_decoder_create failed: %s", opus_strerror(opusErr));
        return false;
    }

    // --- AAudio low-latency output stream ---
    AAudioStreamBuilder* builder = nullptr;
    aaudio_result_t result = AAudio_createStreamBuilder(&builder);
    if (result != AAUDIO_OK) {
        LOGE("AudioPlayer: AAudio_createStreamBuilder failed: %d", result);
        opus_decoder_destroy(m_opusDecoder);
        m_opusDecoder = nullptr;
        return false;
    }

    AAudioStreamBuilder_setSampleRate(builder, sampleRate);
    AAudioStreamBuilder_setChannelCount(builder, channels);
    AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_I16);
    AAudioStreamBuilder_setPerformanceMode(builder, AAUDIO_PERFORMANCE_MODE_LOW_LATENCY);
    AAudioStreamBuilder_setSharingMode(builder, AAUDIO_SHARING_MODE_EXCLUSIVE);
    AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_OUTPUT);

    result = AAudioStreamBuilder_openStream(builder, &m_stream);
    AAudioStreamBuilder_delete(builder);

    if (result != AAUDIO_OK) {
        LOGE("AudioPlayer: AAudioStreamBuilder_openStream failed: %d", result);
        opus_decoder_destroy(m_opusDecoder);
        m_opusDecoder = nullptr;
        return false;
    }

    // Start the stream
    result = AAudioStream_requestStart(m_stream);
    if (result != AAUDIO_OK) {
        LOGE("AudioPlayer: AAudioStream_requestStart failed: %d", result);
        AAudioStream_close(m_stream);
        m_stream = nullptr;
        opus_decoder_destroy(m_opusDecoder);
        m_opusDecoder = nullptr;
        return false;
    }

    // Pre-allocate buffers
    m_decodeBuffer.resize(FRAME_SIZE * channels);
    int maxBufferSamples = (sampleRate * MAX_BUFFER_MS / 1000) * channels;
    m_pcmBuffer.reserve(maxBufferSamples);

    m_initialized = true;
    LOGI("AudioPlayer initialized: %dHz, %d ch, AAudio low-latency", sampleRate, channels);
    return true;
}

void AudioPlayer::shutdown() {
    m_initialized = false;

    if (m_stream) {
        AAudioStream_requestStop(m_stream);
        AAudioStream_close(m_stream);
        m_stream = nullptr;
    }

    if (m_opusDecoder) {
        opus_decoder_destroy(m_opusDecoder);
        m_opusDecoder = nullptr;
    }

    {
        std::lock_guard<std::mutex> lock(m_bufferMutex);
        m_pcmBuffer.clear();
    }

    LOGI("AudioPlayer shut down");
}

bool AudioPlayer::submitOpusPacket(const uint8_t* data, int size) {
    if (!m_initialized || !m_opusDecoder) return false;

    // Decode Opus -> PCM (i16 interleaved)
    int samplesDecoded = opus_decode(
        m_opusDecoder,
        data, size,
        m_decodeBuffer.data(),
        FRAME_SIZE,
        0 // no FEC for now
    );

    if (samplesDecoded < 0) {
        LOGW("AudioPlayer: opus_decode error: %s", opus_strerror(samplesDecoded));
        return false;
    }

    // Append decoded samples to jitter buffer
    int totalSamples = samplesDecoded * m_channels;
    {
        std::lock_guard<std::mutex> lock(m_bufferMutex);

        // Drop oldest samples if buffer is too full (prevents unbounded growth)
        int maxSamples = (m_sampleRate * MAX_BUFFER_MS / 1000) * m_channels;
        if (static_cast<int>(m_pcmBuffer.size()) + totalSamples > maxSamples) {
            int excess = static_cast<int>(m_pcmBuffer.size()) + totalSamples - maxSamples;
            if (excess > 0 && excess <= static_cast<int>(m_pcmBuffer.size())) {
                m_pcmBuffer.erase(m_pcmBuffer.begin(), m_pcmBuffer.begin() + excess);
            }
        }

        m_pcmBuffer.insert(m_pcmBuffer.end(),
            m_decodeBuffer.data(),
            m_decodeBuffer.data() + totalSamples);
    }

    return true;
}

void AudioPlayer::pump() {
    if (!m_initialized || !m_stream) return;

    std::lock_guard<std::mutex> lock(m_bufferMutex);

    if (m_pcmBuffer.empty()) return;

    // Write as many samples as AAudio will accept (non-blocking)
    int framesToWrite = static_cast<int>(m_pcmBuffer.size()) / m_channels;
    aaudio_result_t framesWritten = AAudioStream_write(
        m_stream,
        m_pcmBuffer.data(),
        framesToWrite,
        0 // non-blocking (timeoutNanos = 0)
    );

    if (framesWritten > 0) {
        int samplesWritten = framesWritten * m_channels;
        m_pcmBuffer.erase(m_pcmBuffer.begin(), m_pcmBuffer.begin() + samplesWritten);
    } else if (framesWritten < 0) {
        LOGW("AudioPlayer: AAudioStream_write error: %d", framesWritten);
    }
}
