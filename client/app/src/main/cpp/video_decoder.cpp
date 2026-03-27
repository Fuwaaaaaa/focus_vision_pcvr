#include "video_decoder.h"
#include "xr_utils.h"

#include <GLES2/gl2ext.h> // GL_TEXTURE_EXTERNAL_OES
#include <cstring>

bool VideoDecoder::init(int width, int height, const char* mimeType) {
    m_width = width;
    m_height = height;

    // Create GL_TEXTURE_EXTERNAL_OES for zero-copy Surface output
    glGenTextures(1, &m_outputTexture);
    glBindTexture(GL_TEXTURE_EXTERNAL_OES, m_outputTexture);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_EXTERNAL_OES, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    glBindTexture(GL_TEXTURE_EXTERNAL_OES, 0);

    if (m_outputTexture == 0) {
        LOGE("Failed to create GL_TEXTURE_EXTERNAL_OES");
        return false;
    }

    // Create MediaCodec decoder
    m_codec = AMediaCodec_createDecoderByType(mimeType);
    if (!m_codec) {
        LOGE("Failed to create MediaCodec decoder for %s", mimeType);
        return false;
    }

    m_format = AMediaFormat_new();
    AMediaFormat_setString(m_format, AMEDIAFORMAT_KEY_MIME, mimeType);
    AMediaFormat_setInt32(m_format, AMEDIAFORMAT_KEY_WIDTH, width);
    AMediaFormat_setInt32(m_format, AMEDIAFORMAT_KEY_HEIGHT, height);
    AMediaFormat_setInt32(m_format, AMEDIAFORMAT_KEY_COLOR_FORMAT, 0x7F000789); // COLOR_FormatYUV420Flexible
    // Low latency mode for VR streaming
    AMediaFormat_setInt32(m_format, "low-latency", 1);

    // Configure with Surface for zero-copy output to GL texture.
    // The Surface is created from the GL texture via SurfaceTexture.
    //
    // Note: On Android NDK, creating a Surface from a GL texture requires
    // ASurfaceTexture_* APIs (API level 28+). Focus Vision runs Android 12+
    // so this is available.
    //
    // For the initial implementation, we use buffer output mode as a reliable
    // fallback. Surface output will be added as an optimization.
    media_status_t status = AMediaCodec_configure(m_codec, m_format, nullptr, nullptr, 0);
    if (status != AMEDIA_OK) {
        LOGE("Failed to configure MediaCodec: %d", (int)status);
        AMediaCodec_delete(m_codec);
        m_codec = nullptr;
        return false;
    }

    status = AMediaCodec_start(m_codec);
    if (status != AMEDIA_OK) {
        LOGE("Failed to start MediaCodec: %d", (int)status);
        AMediaCodec_delete(m_codec);
        m_codec = nullptr;
        return false;
    }

    m_initialized = true;
    LOGI("VideoDecoder initialized: %s %dx%d (texture=%u)", mimeType, width, height, m_outputTexture);
    return true;
}

void VideoDecoder::shutdown() {
    if (m_codec) {
        AMediaCodec_stop(m_codec);
        AMediaCodec_delete(m_codec);
        m_codec = nullptr;
    }
    if (m_format) {
        AMediaFormat_delete(m_format);
        m_format = nullptr;
    }
    if (m_outputTexture) {
        glDeleteTextures(1, &m_outputTexture);
        m_outputTexture = 0;
    }
    if (m_surface) {
        ANativeWindow_release(m_surface);
        m_surface = nullptr;
    }
    m_initialized = false;
    LOGI("VideoDecoder shut down");
}

bool VideoDecoder::submitPacket(const uint8_t* data, int size, int64_t timestampUs) {
    if (!m_initialized || !m_codec) return false;

    ssize_t bufIdx = AMediaCodec_dequeueInputBuffer(m_codec, 0); // non-blocking
    if (bufIdx < 0) return false; // no input buffer available

    size_t bufSize;
    uint8_t* buf = AMediaCodec_getInputBuffer(m_codec, bufIdx, &bufSize);
    if (!buf || (size_t)size > bufSize) return false;

    memcpy(buf, data, size);
    AMediaCodec_queueInputBuffer(m_codec, bufIdx, 0, size, timestampUs, 0);
    return true;
}

bool VideoDecoder::getDecodedFrame() {
    if (!m_initialized || !m_codec) return false;

    AMediaCodecBufferInfo info;
    ssize_t bufIdx = AMediaCodec_dequeueOutputBuffer(m_codec, &info, 0); // non-blocking

    if (bufIdx >= 0) {
        // Got a decoded frame.
        // In buffer output mode: read YUV data and upload to GL texture.
        // In Surface output mode: the texture is updated automatically.
        //
        // For now, release the buffer. The GL texture upload path will be
        // added when Surface output is implemented.
        // Setting render=true would update the Surface texture automatically.
        AMediaCodec_releaseOutputBuffer(m_codec, bufIdx, false);
        return true;
    }

    if (bufIdx == AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED) {
        AMediaFormat* newFormat = AMediaCodec_getOutputFormat(m_codec);
        if (newFormat) {
            int32_t w = 0, h = 0;
            AMediaFormat_getInt32(newFormat, AMEDIAFORMAT_KEY_WIDTH, &w);
            AMediaFormat_getInt32(newFormat, AMEDIAFORMAT_KEY_HEIGHT, &h);
            LOGI("VideoDecoder: output format changed to %dx%d", w, h);
            AMediaFormat_delete(newFormat);
        }
    }

    return false;
}

void VideoDecoder::flush() {
    if (!m_initialized || !m_codec) return;
    AMediaCodec_flush(m_codec);
    LOGI("VideoDecoder flushed");
}
