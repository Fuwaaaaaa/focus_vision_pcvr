#include "video_decoder.h"
#include "xr_utils.h"

#include <GLES2/gl2ext.h> // GL_TEXTURE_EXTERNAL_OES
#include <cstring>

bool VideoDecoder::init(int width, int height, const char* mimeType) {
    m_width = width;
    m_height = height;

    // Create GL_TEXTURE_EXTERNAL_OES for Surface output
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

    // Create ASurfaceTexture from the GL texture (API level 28+).
    // This bridges MediaCodec's decoded output directly to our GL texture.
    m_surfaceTexture = ASurfaceTexture_create(m_outputTexture);
    if (m_surfaceTexture) {
        m_surface = ASurfaceTexture_acquireANativeWindow(m_surfaceTexture);
        if (m_surface) {
            m_useSurfaceOutput = true;
            LOGI("VideoDecoder: Surface output enabled (zero-copy)");
        } else {
            LOGW("VideoDecoder: Failed to acquire ANativeWindow, falling back to buffer mode");
            ASurfaceTexture_release(m_surfaceTexture);
            m_surfaceTexture = nullptr;
        }
    } else {
        LOGW("VideoDecoder: ASurfaceTexture_create failed, falling back to buffer mode");
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
    // Low latency mode for VR streaming
    AMediaFormat_setInt32(m_format, "low-latency", 1);

    // Configure with Surface if available (zero-copy path), otherwise buffer mode
    media_status_t status = AMediaCodec_configure(
        m_codec, m_format,
        m_useSurfaceOutput ? m_surface : nullptr,
        nullptr, 0);
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
    LOGI("VideoDecoder initialized: %s %dx%d (texture=%u, surface=%s)",
         mimeType, width, height, m_outputTexture,
         m_useSurfaceOutput ? "yes" : "no");
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
    if (m_surface) {
        ANativeWindow_release(m_surface);
        m_surface = nullptr;
    }
    if (m_surfaceTexture) {
        ASurfaceTexture_release(m_surfaceTexture);
        m_surfaceTexture = nullptr;
    }
    if (m_outputTexture) {
        glDeleteTextures(1, &m_outputTexture);
        m_outputTexture = 0;
    }
    m_useSurfaceOutput = false;
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
        if (m_useSurfaceOutput) {
            // Surface output: release with render=true to update the SurfaceTexture.
            // Then call updateTexImage() to make the new frame available in the GL texture.
            AMediaCodec_releaseOutputBuffer(m_codec, bufIdx, true);
            ASurfaceTexture_updateTexImage(m_surfaceTexture);
        } else {
            // Buffer output: release without rendering.
            // In this fallback path, the GL texture stays empty.
            // A full buffer-to-texture upload would be needed here for production.
            AMediaCodec_releaseOutputBuffer(m_codec, bufIdx, false);
        }
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
