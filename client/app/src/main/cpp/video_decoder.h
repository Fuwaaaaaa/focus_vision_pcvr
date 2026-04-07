#pragma once

#include <cstdint>
#include <jni.h>
#include <GLES3/gl3.h>
#include <media/NdkMediaCodec.h>
#include <media/NdkMediaFormat.h>
#include <android/native_window.h>
#include <android/surface_texture.h>
#include <android/surface_texture_jni.h>
#include <atomic>
#include <chrono>
#include <deque>

/// Hardware video decoder using Android MediaCodec NDK API.
/// Decodes H.265/H.264 NAL units to GL_TEXTURE_EXTERNAL_OES via Surface output.
class VideoDecoder {
public:
    /// Initialize the decoder. Pass a JNIEnv* to enable zero-copy SurfaceTexture output.
    /// If env is nullptr, falls back to buffer output mode.
    bool init(JNIEnv* env, int width, int height, const char* mimeType = "video/hevc");
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

    /// Get the rolling average decode latency in microseconds.
    uint32_t avgDecodeLatencyUs() const { return m_avgDecodeUs; }

private:
    void cleanupSurfaceResources(JNIEnv* env);

    AMediaCodec* m_codec = nullptr;
    AMediaFormat* m_format = nullptr;
    std::atomic<bool> m_initialized{false};
    int m_width = 0;
    int m_height = 0;

    // Surface output for zero-copy decode to GL texture.
    // ASurfaceTexture wraps the GL texture so MediaCodec renders directly to it.
    GLuint m_outputTexture = 0;
    ASurfaceTexture* m_surfaceTexture = nullptr;
    ANativeWindow* m_surface = nullptr;
    bool m_useSurfaceOutput = false;

    // JNI references for SurfaceTexture lifecycle
    JavaVM* m_javaVM = nullptr;
    jobject m_javaSurfaceTexture = nullptr;

    // Decode latency measurement (submit-to-output wall time)
    std::deque<std::chrono::steady_clock::time_point> m_submitTimes;
    uint32_t m_avgDecodeUs = 0;
    uint32_t m_decodeCount = 0;
    uint64_t m_totalDecodeUs = 0;
};
