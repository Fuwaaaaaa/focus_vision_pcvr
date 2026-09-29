#pragma once

#include <GLES3/gl3.h>
#include <cstdint>

#include "video_view.h"

/// OpenGL ES renderer for decoded video frames.
class Renderer {
public:
    void init();
    void shutdown();

    /// Render a solid color to the given framebuffer.
    void renderSolidColor(GLuint framebuffer, uint32_t width, uint32_t height,
                          float r, float g, float b);

    /// Render one eye of a decoded frame (external OES texture from
    /// MediaCodec) to the framebuffer: the eye's region of the frame
    /// (`eye`), reprojected by `rotation` (row-major, current view → the
    /// frame's; fvp_video::kIdentity for a new frame) over the eye's field
    /// of view. Directions outside the frame are drawn black.
    void renderVideoFrame(GLuint framebuffer, uint32_t width, uint32_t height,
                          GLuint videoTexture, const fvp_video::UvRect& eye,
                          const float rotation[9], const fvp_video::Tangents& fov);

private:
    bool m_initialized = false;

    // Shader program for rendering external OES textures (from MediaCodec)
    GLuint m_videoProgram = 0;
    GLuint m_vao = 0;
    GLuint m_vbo = 0;
    GLint m_uTexture = -1;
    GLint m_uRotation = -1;
    GLint m_uTangents = -1;
    GLint m_uEyeRect = -1;

    bool createVideoShader();
};
