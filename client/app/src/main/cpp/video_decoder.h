#pragma once

#include <cstdint>
#include <GLES3/gl3.h>
#include <media/NdkMediaCodec.h>
#include <media/NdkMediaFormat.h>
#include <android/native_window.h>
#include <atomic>

/// Hardware video decoder using Android MediaCodec NDK API.
/// Decodes H.265/H.264 NAL units to GL_TEXTURE_EXTERNAL_OES via Surface output.
class VideoDecoder {
public:
    bool init(int width, int height, const char* mimeType = "video/hevc");
    void shutdown();

    /// Submit encoded NAL data to the decoder. Returns true on success.
    bool submitPacket(const uint8_t* data, int size, int64_t timestampUs);

    /// Try to get a decoded frame. Returns true if a frame was output.
    /// After this returns true, getOutputTexture() provides the GL texture.
    bool getDecodedFrame();

    /// Get the GL_TEXTURE_EXTERNAL_OES texture containing the decoded frame.
    GLuint getOutputTexture() const { return m_outputTexture; }

    /// Flush the decoder (e.g., after seeking or IDR request).
    void flush();

    bool isInitialized() const { return m_initialized; }

private:
    AMediaCodec* m_codec = nullptr;
    AMediaFormat* m_format = nullptr;
    std::atomic<bool> m_initialized{false};
    int m_width = 0;
    int m_height = 0;

    // Surface output for zero-copy decode to GL texture
    GLuint m_outputTexture = 0;
    ANativeWindow* m_surface = nullptr;
};
