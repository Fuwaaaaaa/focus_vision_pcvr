#include "overlay_renderer.h"
#include "xr_utils.h"
#include <cmath>

static const char* kOverlayVS = R"(#version 300 es
layout(location = 0) in vec2 aPos;
uniform vec2 uOffset;
uniform vec2 uScale;
void main() {
    gl_Position = vec4(aPos * uScale + uOffset, 0.0, 1.0);
}
)";

static const char* kOverlayFS = R"(#version 300 es
precision mediump float;
uniform vec3 uColor;
out vec4 fragColor;
void main() {
    fragColor = vec4(uColor, 0.8);
}
)";

static GLuint compileShader(GLenum type, const char* src) {
    GLuint s = glCreateShader(type);
    glShaderSource(s, 1, &src, nullptr);
    glCompileShader(s);
    return s;
}

void OverlayRenderer::init() {
    GLuint vs = compileShader(GL_VERTEX_SHADER, kOverlayVS);
    GLuint fs = compileShader(GL_FRAGMENT_SHADER, kOverlayFS);
    m_program = glCreateProgram();
    glAttachShader(m_program, vs);
    glAttachShader(m_program, fs);
    glLinkProgram(m_program);
    glDeleteShader(vs);
    glDeleteShader(fs);

    // Unit quad
    float quad[] = { 0, 0, 1, 0, 0, 1, 1, 1 };
    glGenVertexArrays(1, &m_vao);
    glGenBuffers(1, &m_vbo);
    glBindVertexArray(m_vao);
    glBindBuffer(GL_ARRAY_BUFFER, m_vbo);
    glBufferData(GL_ARRAY_BUFFER, sizeof(quad), quad, GL_STATIC_DRAW);
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, nullptr);
    glBindVertexArray(0);

    m_initialized = true;
    LOGI("OverlayRenderer initialized");
}

void OverlayRenderer::shutdown() {
    if (m_program) glDeleteProgram(m_program);
    if (m_vbo) glDeleteBuffers(1, &m_vbo);
    if (m_vao) glDeleteVertexArrays(1, &m_vao);
    m_initialized = false;
}

void OverlayRenderer::renderBar(float x, float y, float width, float height,
                                 float r, float g, float b) {
    glUniform2f(glGetUniformLocation(m_program, "uOffset"), x, y);
    glUniform2f(glGetUniformLocation(m_program, "uScale"), width, height);
    glUniform3f(glGetUniformLocation(m_program, "uColor"), r, g, b);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
}

void OverlayRenderer::render(GLuint framebuffer, uint32_t fbWidth, uint32_t fbHeight,
                              float quality, float packetLossPercent, float latencyMs) {
    if (!m_initialized) return;
    (void)fbWidth; (void)fbHeight;

    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glUseProgram(m_program);
    glBindVertexArray(m_vao);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    // Color based on quality: green (good) → yellow (degraded) → red (poor)
    float r, g, b;
    if (quality > 0.7f) {
        r = 0.2f; g = 0.83f; b = 0.6f; // Emerald green (#34D399)
    } else if (quality > 0.4f) {
        r = 0.98f; g = 0.75f; b = 0.14f; // Yellow
    } else {
        r = 0.97f; g = 0.44f; b = 0.44f; // Red
    }

    // Draw 3 signal bars in bottom-left corner (NDC: -1 to 1)
    float baseX = -0.95f;
    float baseY = -0.95f;
    float barWidth = 0.02f;
    float gap = 0.01f;

    // Bar 1 (short) — always visible
    renderBar(baseX, baseY, barWidth, 0.04f, r, g, b);
    // Bar 2 (medium) — visible if quality > 0.4
    float bar2Alpha = quality > 0.4f ? 1.0f : 0.2f;
    renderBar(baseX + barWidth + gap, baseY, barWidth, 0.07f,
              r * bar2Alpha, g * bar2Alpha, b * bar2Alpha);
    // Bar 3 (tall) — visible if quality > 0.7
    float bar3Alpha = quality > 0.7f ? 1.0f : 0.2f;
    renderBar(baseX + 2.0f * (barWidth + gap), baseY, barWidth, 0.10f,
              r * bar3Alpha, g * bar3Alpha, b * bar3Alpha);

    glDisable(GL_BLEND);
    glBindVertexArray(0);
    glUseProgram(0);
}

void OverlayRenderer::renderLatencyWaterfall(GLuint framebuffer, uint32_t fbWidth, uint32_t fbHeight,
                                              uint32_t encodeUs, uint32_t networkUs,
                                              uint32_t decodeUs, uint32_t renderUs) {
    if (!m_initialized) return;
    (void)fbWidth; (void)fbHeight;

    uint32_t totalUs = encodeUs + networkUs + decodeUs + renderUs;
    if (totalUs == 0) return;

    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glUseProgram(m_program);
    glBindVertexArray(m_vao);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    // Waterfall position: bottom-left, above signal bars
    float baseX = -0.95f;
    float baseY = -0.82f;
    float totalWidth = 0.40f;
    float barHeight = 0.025f;

    // Stacked horizontal bars: Encode (blue) | Network (cyan) | Decode (green) | Render (yellow)
    float x = baseX;
    float encW = totalWidth * (float)encodeUs / (float)totalUs;
    float netW = totalWidth * (float)networkUs / (float)totalUs;
    float decW = totalWidth * (float)decodeUs / (float)totalUs;
    float renW = totalWidth * (float)renderUs / (float)totalUs;

    renderBar(x, baseY, encW, barHeight, 0.37f, 0.51f, 0.89f);  // Blue — encode
    x += encW;
    renderBar(x, baseY, netW, barHeight, 0.26f, 0.83f, 0.96f);  // Cyan — network
    x += netW;
    renderBar(x, baseY, decW, barHeight, 0.2f, 0.83f, 0.6f);    // Green — decode
    x += decW;
    renderBar(x, baseY, renW, barHeight, 0.98f, 0.75f, 0.14f);  // Yellow — render

    glDisable(GL_BLEND);
    glBindVertexArray(0);
    glUseProgram(0);
}

void OverlayRenderer::renderDashboard(GLuint framebuffer, uint32_t fbWidth, uint32_t fbHeight,
                                       uint32_t bitrateMbps, bool codecIsH265, int highlightItem) {
    if (!m_initialized || !m_dashboardVisible) return;

    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glViewport(0, 0, fbWidth, fbHeight);
    glUseProgram(m_program);
    glBindVertexArray(m_vao);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    // Panel background (bottom-right, semi-transparent dark)
    // NDC coordinates: x=0.3..0.95, y=-0.95..-0.5
    float panelX = 0.3f;
    float panelY = -0.95f;
    float panelW = 0.65f;
    float panelH = 0.45f;
    renderBar(panelX, panelY, panelW, panelH, 0.04f, 0.04f, 0.05f); // bg-primary

    // Bitrate label area
    float itemY = panelY + panelH - 0.12f;
    float itemX = panelX + 0.05f;

    // "[-]" button
    float btnColor = (highlightItem == 0) ? 0.83f : 0.4f;
    renderBar(itemX, itemY, 0.08f, 0.08f, 0.1f, btnColor * 0.5f, btnColor * 0.3f);

    // Bitrate value area
    renderBar(itemX + 0.12f, itemY, 0.25f, 0.08f, 0.07f, 0.07f, 0.08f);

    // "[+]" button
    btnColor = (highlightItem == 2) ? 0.83f : 0.4f;
    renderBar(itemX + 0.42f, itemY, 0.08f, 0.08f, 0.1f, btnColor * 0.5f, btnColor * 0.3f);

    // Codec indicator row
    float codecY = itemY - 0.14f;
    float codecColor = codecIsH265 ? 0.2f : 0.6f;
    renderBar(itemX, codecY, 0.25f, 0.08f, 0.07f, codecColor * 0.3f, 0.07f);
    float h264Color = !codecIsH265 ? 0.2f : 0.6f;
    renderBar(itemX + 0.28f, codecY, 0.25f, 0.08f, 0.07f, h264Color * 0.3f, 0.07f);

    glDisable(GL_BLEND);
    glBindVertexArray(0);
    glUseProgram(0);
}

void OverlayRenderer::renderSleepDimming(GLuint framebuffer, uint32_t fbWidth, uint32_t fbHeight, float alpha) {
    if (!m_initialized || alpha <= 0.01f) return;

    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glViewport(0, 0, fbWidth, fbHeight);
    glUseProgram(m_program);
    glBindVertexArray(m_vao);
    glEnable(GL_BLEND);
    glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

    // Full-screen black quad with variable alpha
    renderBar(-1.0f, -1.0f, 2.0f, 2.0f, 0.0f, 0.0f, 0.0f);

    glDisable(GL_BLEND);
    glBindVertexArray(0);
    glUseProgram(0);
}
