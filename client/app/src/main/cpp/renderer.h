#pragma once

#include <GLES3/gl3.h>
#include <cstdint>

/// OpenGL ES renderer for decoded video frames.
class Renderer {
public:
    void init();
    void shutdown();

    /// Render a solid color to the given framebuffer.
    void renderSolidColor(GLuint framebuffer, uint32_t width, uint32_t height,
                          float r, float g, float b);

    /// Render a video frame (external OES texture from MediaCodec) to the framebuffer.
    void renderVideoFrame(GLuint framebuffer, uint32_t width, uint32_t height,
                          GLuint videoTexture);

private:
    bool m_initialized = false;

    // Shader program for rendering external OES textures (from MediaCodec)
    GLuint m_videoProgram = 0;
    GLuint m_vao = 0;
    GLuint m_vbo = 0;

    bool createVideoShader();
};
