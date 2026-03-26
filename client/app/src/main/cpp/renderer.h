#pragma once

#include <GLES3/gl3.h>
#include <cstdint>

/// OpenGL ES renderer. Step 4: solid color. Step 5: video frames.
class Renderer {
public:
    void init();
    void shutdown();

    /// Render a solid color to the given framebuffer.
    void renderSolidColor(GLuint framebuffer, uint32_t width, uint32_t height,
                          float r, float g, float b);

    // Step 5: void renderVideoFrame(GLuint framebuffer, uint32_t width, uint32_t height,
    //                               GLuint videoTexture);

private:
    bool m_initialized = false;
};
