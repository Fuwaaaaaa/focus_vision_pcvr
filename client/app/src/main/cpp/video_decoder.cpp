#include "video_decoder.h"
#include "xr_utils.h"

bool VideoDecoder::init(int width, int height, const char* mimeType) {
    m_width = width;
    m_height = height;

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
    // Low latency mode
    AMediaFormat_setInt32(m_format, "low-latency", 1);

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
    LOGI("VideoDecoder initialized: %s %dx%d", mimeType, width, height);
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
        // Got a decoded frame
        // Step 5 will: read the output buffer, convert to GL texture
        AMediaCodec_releaseOutputBuffer(m_codec, bufIdx, false);
        return true;
    }

    return false;
}
