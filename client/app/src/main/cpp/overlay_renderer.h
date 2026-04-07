#pragma once

#include <GLES3/gl3.h>
#include <cstdint>

/// Renders a small connection quality indicator overlay in the HMD.
/// Drawn as a simple colored bar icon (Wi-Fi signal strength style).
/// Green = good, yellow = degraded, red = poor.
class OverlayRenderer {
public:
    void init();
    void shutdown();

    /// Render the quality indicator to a framebuffer.
    /// Quality is 0.0 (worst) to 1.0 (best).
    void render(GLuint framebuffer, uint32_t fbWidth, uint32_t fbHeight,
                float quality, float packetLossPercent, float latencyMs);

    /// Render a full-screen dimming overlay for sleep mode.
    /// alpha: 0.0 = fully transparent, 1.0 = fully black.
    void renderSleepDimming(GLuint framebuffer, uint32_t fbWidth, uint32_t fbHeight, float alpha);

private:
    GLuint m_program = 0;
    GLuint m_vao = 0;
    GLuint m_vbo = 0;
    bool m_initialized = false;

    void renderBar(float x, float y, float width, float height,
                   float r, float g, float b);
};
