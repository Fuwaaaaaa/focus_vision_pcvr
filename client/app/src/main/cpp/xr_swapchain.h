#pragma once

#include <openxr/openxr.h>
#include <GLES3/gl3.h>
#include <vector>

/// Wraps an OpenXR swapchain with GL framebuffers for rendering.
class XrSwapchainWrapper {
public:
    XrSwapchainWrapper() = default;
    ~XrSwapchainWrapper() = default;

    void create(XrSession session, uint32_t width, uint32_t height);
    void destroy();

    void acquireImage(uint32_t* outIndex);
    void waitImage();
    void releaseImage();

    XrSwapchain getHandle() const { return m_swapchain; }
    GLuint getFramebuffer(uint32_t index) const { return m_framebuffers[index]; }
    uint32_t getWidth() const { return m_width; }
    uint32_t getHeight() const { return m_height; }

private:
    XrSwapchain m_swapchain = XR_NULL_HANDLE;
    uint32_t m_width = 0;
    uint32_t m_height = 0;

    std::vector<XrSwapchainImageOpenGLESKHR> m_images;
    std::vector<GLuint> m_framebuffers;
};
