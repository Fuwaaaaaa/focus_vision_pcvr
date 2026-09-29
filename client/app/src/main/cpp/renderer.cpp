#include "renderer.h"
#include "xr_utils.h"

#include <GLES2/gl2ext.h> // GL_TEXTURE_EXTERNAL_OES

// Vertex shader: full-screen quad; the fragment shader works from the
// pixel's position in normalized device coordinates.
static const char* kVertexShader = R"(#version 300 es
layout(location = 0) in vec2 aPos;
out vec2 vNdc;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
    vNdc = aPos;
}
)";

// Fragment shader: the pixel's direction in the current view, rotated into
// the view the frame was drawn for, projected onto the frame, and looked up
// in this eye's part of the decoded texture. Per pixel, because the mapping
// is projective (a per-vertex divide would bend straight lines).
// fvp_video::sampleUv / frameUv are the same math, tested on the host.
static const char* kFragmentShader = R"(#version 300 es
#extension GL_OES_EGL_image_external_essl3 : require
precision highp float;
in vec2 vNdc;
out vec4 fragColor;
uniform samplerExternalOES uTexture;
uniform mat3 uRotation;  // current view -> the frame's view
uniform vec4 uTangents;  // left, right, down, up
uniform vec4 uEyeRect;   // u0, v0, u1, v1 of this eye in the frame
void main() {
    // OpenXR views look down -Z.
    vec2 t = mix(uTangents.xz, uTangents.yw, vNdc * 0.5 + 0.5);
    vec3 d = uRotation * vec3(t, -1.0);
    if (d.z >= -1e-6) { fragColor = vec4(0.0, 0.0, 0.0, 1.0); return; }
    vec2 uv = (d.xy / -d.z - uTangents.xz) / (uTangents.yw - uTangents.xz);
    uv.y = 1.0 - uv.y;  // image top at v = 0
    if (any(lessThan(uv, vec2(0.0))) || any(greaterThan(uv, vec2(1.0)))) {
        fragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }
    fragColor = texture(uTexture, mix(uEyeRect.xy, uEyeRect.zw, uv));
}
)";

// Full-screen quad (triangle strip)
static const float kQuadVertices[] = {
    -1.0f, -1.0f,
     1.0f, -1.0f,
    -1.0f,  1.0f,
     1.0f,  1.0f,
};

static GLuint compileShader(GLenum type, const char* source) {
    GLuint shader = glCreateShader(type);
    glShaderSource(shader, 1, &source, nullptr);
    glCompileShader(shader);

    GLint compiled = 0;
    glGetShaderiv(shader, GL_COMPILE_STATUS, &compiled);
    if (!compiled) {
        char log[512];
        glGetShaderInfoLog(shader, sizeof(log), nullptr, log);
        LOGE("Shader compile error: %s", log);
        glDeleteShader(shader);
        return 0;
    }
    return shader;
}

void Renderer::init() {
    createVideoShader();
    LOGI("Renderer initialized (GLES %s)", glGetString(GL_VERSION));
    m_initialized = true;
}

void Renderer::shutdown() {
    if (m_videoProgram) {
        glDeleteProgram(m_videoProgram);
        m_videoProgram = 0;
    }
    if (m_vao) {
        glDeleteVertexArrays(1, &m_vao);
        m_vao = 0;
    }
    if (m_vbo) {
        glDeleteBuffers(1, &m_vbo);
        m_vbo = 0;
    }
    m_initialized = false;
}

void Renderer::renderSolidColor(GLuint framebuffer, uint32_t width, uint32_t height,
                                 float r, float g, float b) {
    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glViewport(0, 0, width, height);
    glClearColor(r, g, b, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT);
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
}

void Renderer::renderVideoFrame(GLuint framebuffer, uint32_t width, uint32_t height,
                                 GLuint videoTexture, const fvp_video::UvRect& eye,
                                 const float rotation[9], const fvp_video::Tangents& fov) {
    if (!m_videoProgram || !m_vao) return;

    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glViewport(0, 0, width, height);
    glDisable(GL_DEPTH_TEST);

    glUseProgram(m_videoProgram);

    // Bind the external OES texture from MediaCodec
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_EXTERNAL_OES, videoTexture);
    glUniform1i(m_uTexture, 0);
    // Row-major, so transposed on upload (GLSL matrices are column-major).
    glUniformMatrix3fv(m_uRotation, 1, GL_TRUE, rotation);
    glUniform4f(m_uTangents, fov.left, fov.right, fov.down, fov.up);
    glUniform4f(m_uEyeRect, eye.u0, eye.v0, eye.u1, eye.v1);

    // Draw full-screen quad
    glBindVertexArray(m_vao);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    glBindVertexArray(0);

    glUseProgram(0);
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
}

bool Renderer::createVideoShader() {
    GLuint vs = compileShader(GL_VERTEX_SHADER, kVertexShader);
    GLuint fs = compileShader(GL_FRAGMENT_SHADER, kFragmentShader);
    if (!vs || !fs) return false;

    m_videoProgram = glCreateProgram();
    glAttachShader(m_videoProgram, vs);
    glAttachShader(m_videoProgram, fs);
    glLinkProgram(m_videoProgram);

    glDeleteShader(vs);
    glDeleteShader(fs);

    GLint linked = 0;
    glGetProgramiv(m_videoProgram, GL_LINK_STATUS, &linked);
    if (!linked) {
        char log[512];
        glGetProgramInfoLog(m_videoProgram, sizeof(log), nullptr, log);
        LOGE("Program link error: %s", log);
        glDeleteProgram(m_videoProgram);
        m_videoProgram = 0;
        return false;
    }
    m_uTexture = glGetUniformLocation(m_videoProgram, "uTexture");
    m_uRotation = glGetUniformLocation(m_videoProgram, "uRotation");
    m_uTangents = glGetUniformLocation(m_videoProgram, "uTangents");
    m_uEyeRect = glGetUniformLocation(m_videoProgram, "uEyeRect");

    // Create VAO + VBO for full-screen quad
    glGenVertexArrays(1, &m_vao);
    glGenBuffers(1, &m_vbo);

    glBindVertexArray(m_vao);
    glBindBuffer(GL_ARRAY_BUFFER, m_vbo);
    glBufferData(GL_ARRAY_BUFFER, sizeof(kQuadVertices), kQuadVertices, GL_STATIC_DRAW);

    // Position attribute
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 2 * sizeof(float), (void*)0);

    glBindVertexArray(0);
    return true;
}
