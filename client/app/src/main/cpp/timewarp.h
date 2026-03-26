#pragma once

#include <openxr/openxr.h>
#include <GLES3/gl3.h>
#include <cstdint>

/**
 * Basic Timewarp (Late-Stage Reprojection).
 *
 * When a new decoded frame hasn't arrived in time, takes the last decoded frame
 * and applies a 2D rotation correction based on the delta between the render pose
 * (when the frame was drawn on PC) and the current head pose (latest from OpenXR).
 *
 * This prevents VR sickness from displaying stale pose data.
 *
 * Limitation: 2D rotation only (no parallax correction for translation).
 * Full 3D ATW deferred to v2.
 */
class Timewarp {
public:
    bool init();
    void shutdown();

    /**
     * Apply timewarp: re-render the given texture with rotation correction.
     *
     * @param targetFBO   Framebuffer to render into (swapchain image)
     * @param width       Target framebuffer width
     * @param height      Target framebuffer height
     * @param sourceTexture  The last decoded video frame texture
     * @param renderPose  The head pose when this frame was rendered on PC
     * @param currentPose The current head pose (predicted for display time)
     * @param fov         The eye's field of view (for projection matrix)
     */
    void apply(GLuint targetFBO, uint32_t width, uint32_t height,
               GLuint sourceTexture,
               const XrPosef& renderPose,
               const XrPosef& currentPose,
               const XrFovf& fov);

private:
    bool compileShaders();
    void computeUVTransform(const XrPosef& renderPose, const XrPosef& currentPose,
                            const XrFovf& fov, float outMatrix[9]);

    // Quaternion helpers
    static void quatInverse(const XrQuaternionf& q, XrQuaternionf& out);
    static void quatMultiply(const XrQuaternionf& a, const XrQuaternionf& b, XrQuaternionf& out);
    static void quatToMatrix3(const XrQuaternionf& q, float out[9]);

    GLuint m_program = 0;
    GLuint m_vao = 0;
    GLuint m_vbo = 0;

    // Uniform locations
    GLint m_uTexture = -1;
    GLint m_uUVTransform = -1;

    bool m_initialized = false;
};
