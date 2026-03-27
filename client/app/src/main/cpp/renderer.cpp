#include "renderer.h"
#include "xr_utils.h"

#include <GLES2/gl2ext.h> // GL_TEXTURE_EXTERNAL_OES

// Vertex shader: full-screen quad
static const char* kVertexShader = R"(#version 300 es
layout(location = 0) in vec2 aPos;
layout(location = 1) in vec2 aTexCoord;
out vec2 vTexCoord;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
    vTexCoord = aTexCoord;
}
)";

// Fragment shader: sample from external OES texture (MediaCodec output)
static const char* kFragmentShader = R"(#version 300 es
#extension GL_OES_EGL_image_external_essl3 : require
precision mediump float;
in vec2 vTexCoord;
out vec4 fragColor;
uniform samplerExternalOES uTexture;
void main() {
    fragColor = texture(uTexture, vTexCoord);
}
)";

// Full-screen quad: position (x,y) + texcoord (u,v)
static const float kQuadVertices[] = {
    // pos       // uv
    -1.0f, -1.0f,  0.0f, 1.0f,  // bottom-left (flip Y for Android)
     1.0f, -1.0f,  1.0f, 1.0f,  // bottom-right
    -1.0f,  1.0f,  0.0f, 0.0f,  // top-left
     1.0f,  1.0f,  1.0f, 0.0f,  // top-right
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
                                 GLuint videoTexture) {
    if (!m_videoProgram || !m_vao) return;

    glBindFramebuffer(GL_FRAMEBUFFER, framebuffer);
    glViewport(0, 0, width, height);

    glUseProgram(m_videoProgram);

    // Bind the external OES texture from MediaCodec
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_EXTERNAL_OES, videoTexture);
    glUniform1i(glGetUniformLocation(m_videoProgram, "uTexture"), 0);

    // Draw full-screen quad
    glBindVertexArray(m_vao);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    glBindVertexArray(0);

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

    // Create VAO + VBO for full-screen quad
    glGenVertexArrays(1, &m_vao);
    glGenBuffers(1, &m_vbo);

    glBindVertexArray(m_vao);
    glBindBuffer(GL_ARRAY_BUFFER, m_vbo);
    glBufferData(GL_ARRAY_BUFFER, sizeof(kQuadVertices), kQuadVertices, GL_STATIC_DRAW);

    // Position attribute
    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (void*)0);
    // TexCoord attribute
    glEnableVertexAttribArray(1);
    glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 4 * sizeof(float), (void*)(2 * sizeof(float)));

    glBindVertexArray(0);
    return true;
}
