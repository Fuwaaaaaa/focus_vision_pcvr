#pragma once

#include <cstdint>
#include <media/NdkMediaCodec.h>
#include <media/NdkMediaFormat.h>
#include <atomic>

/// Hardware video decoder using Android MediaCodec NDK API.
/// Stub in Step 4, full implementation in Step 5.
class VideoDecoder {
public:
    bool init(int width, int height, const char* mimeType = "video/hevc");
    void shutdown();

    /// Submit encoded data to the decoder. Returns true on success.
    bool submitPacket(const uint8_t* data, int size, int64_t timestampUs);

    /// Try to get a decoded frame. Returns true if a frame was output.
    /// The decoded frame is available as a GL texture via getOutputTexture().
    bool getDecodedFrame();

    bool isInitialized() const { return m_initialized; }

private:
    AMediaCodec* m_codec = nullptr;
    AMediaFormat* m_format = nullptr;
    std::atomic<bool> m_initialized{false};
    int m_width = 0;
    int m_height = 0;
};
