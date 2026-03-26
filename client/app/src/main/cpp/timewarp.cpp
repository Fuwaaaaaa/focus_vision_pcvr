#include "timewarp.h"
#include "xr_utils.h"
#include <cmath>
#include <cstring>

// Vertex shader: fullscreen quad with UV transform for rotation correction
static const char* kVertexShader = R"(#version 300 es
layout(location = 0) in vec2 a_position;
uniform mat3 u_uvTransform;
out vec2 v_texCoord;
void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
    vec2 uv = a_position * 0.5 + 0.5;
    vec3 transformed = u_uvTransform * vec3(uv * 2.0 - 1.0, 1.0);
    v_texCoord = (transformed.xy / transformed.z) * 0.5 + 0.5;
}
)";

// Fragment shader: sample the video texture
static const char* kFragmentShader = R"(#version 300 es
precision mediump float;
in vec2 v_texCoord;
uniform sampler2D u_texture;
out vec4 fragColor;
void main() {
    // Clamp UVs to avoid sampling outside the texture
    vec2 uv = clamp(v_texCoord, 0.0, 1.0);
    fragColor = texture(u_texture, uv);
}
)";

// Fullscreen quad vertices (two triangles)
static const float kQuadVertices[] = {
    -1.0f, -1.0f,
     1.0f, -1.0f,
    -1.0f,  1.0f,
     1.0f, -1.0f,
     1.0f,  1.0f,
    -1.0f,  1.0f,
};

bool Timewarp::init() {
    if (!compileShaders()) return false;

    // Create VAO and VBO for the fullscreen quad
    glGenVertexArrays(1, &m_vao);
    glGenBuffers(1, &m_vbo);

    glBindVertexArray(m_vao);
    glBindBuffer(GL_ARRAY_BUFFER, m_vbo);
    glBufferData(GL_ARRAY_BUFFER, sizeof(kQuadVertices), kQuadVertices, GL_STATIC_DRAW);

    glEnableVertexAttribArray(0);
    glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 2 * sizeof(float), nullptr);

    glBindVertexArray(0);

    m_initialized = true;
    LOGI("Timewarp initialized");
    return true;
}

void Timewarp::shutdown() {
    if (m_vao) { glDeleteVertexArrays(1, &m_vao); m_vao = 0; }
    if (m_vbo) { glDeleteBuffers(1, &m_vbo); m_vbo = 0; }
    if (m_program) { glDeleteProgram(m_program); m_program = 0; }
    m_initialized = false;
}

void Timewarp::apply(GLuint targetFBO, uint32_t width, uint32_t height,
                      GLuint sourceTexture,
                      const XrPosef& renderPose,
                      const XrPosef& currentPose,
                      const XrFovf& fov) {
    if (!m_initialized) return;

    // Compute UV transform matrix
    float uvTransform[9];
    computeUVTransform(renderPose, currentPose, fov, uvTransform);

    // Render
    glBindFramebuffer(GL_FRAMEBUFFER, targetFBO);
    glViewport(0, 0, width, height);
    glDisable(GL_DEPTH_TEST);

    glUseProgram(m_program);

    // Bind source texture
    glActiveTexture(GL_TEXTURE0);
    glBindTexture(GL_TEXTURE_2D, sourceTexture);
    glUniform1i(m_uTexture, 0);

    // Set UV transform
    glUniformMatrix3fv(m_uUVTransform, 1, GL_FALSE, uvTransform);

    // Draw fullscreen quad
    glBindVertexArray(m_vao);
    glDrawArrays(GL_TRIANGLES, 0, 6);

    glBindVertexArray(0);
    glUseProgram(0);
    glBindFramebuffer(GL_FRAMEBUFFER, 0);
}

void Timewarp::computeUVTransform(const XrPosef& renderPose,
                                    const XrPosef& currentPose,
                                    const XrFovf& fov,
                                    float out[9]) {
    // delta = current * inverse(render)
    XrQuaternionf renderInv;
    quatInverse(renderPose.orientation, renderInv);

    XrQuaternionf delta;
    quatMultiply(currentPose.orientation, renderInv, delta);

    // Convert delta rotation to 3x3 matrix
    float rotMatrix[9];
    quatToMatrix3(delta, rotMatrix);

    // Build simple projection-space UV transform
    // For small rotations this is approximately: identity + rotation offset
    // Full version: P * R * P^-1, but for basic timewarp we use rotation directly
    float tanL = tanf(fov.angleLeft);
    float tanR = tanf(fov.angleRight);
    float tanU = tanf(fov.angleUp);
    float tanD = tanf(fov.angleDown);

    float scaleX = 2.0f / (tanR - tanL);
    float scaleY = 2.0f / (tanU - tanD);

    // Simplified UV transform: scale → rotate → unscale
    // P = diag(scaleX, scaleY, 1)
    // result = P * rot * P^-1 ≈ rot for small angles
    // For large angles, the full P*R*P^-1 gives better results
    float P[9] = {scaleX, 0, 0,  0, scaleY, 0,  0, 0, 1};
    float Pinv[9] = {1.0f/scaleX, 0, 0,  0, 1.0f/scaleY, 0,  0, 0, 1};

    // out = P * rot * Pinv
    float temp[9];
    // temp = rot * Pinv
    for (int r = 0; r < 3; r++) {
        for (int c = 0; c < 3; c++) {
            temp[r*3+c] = 0;
            for (int k = 0; k < 3; k++) {
                temp[r*3+c] += rotMatrix[r*3+k] * Pinv[k*3+c];
            }
        }
    }
    // out = P * temp
    for (int r = 0; r < 3; r++) {
        for (int c = 0; c < 3; c++) {
            out[r*3+c] = 0;
            for (int k = 0; k < 3; k++) {
                out[r*3+c] += P[r*3+k] * temp[k*3+c];
            }
        }
    }
}

bool Timewarp::compileShaders() {
    auto compile = [](GLenum type, const char* src) -> GLuint {
        GLuint shader = glCreateShader(type);
        glShaderSource(shader, 1, &src, nullptr);
        glCompileShader(shader);
        GLint ok;
        glGetShaderiv(shader, GL_COMPILE_STATUS, &ok);
        if (!ok) {
            char log[512];
            glGetShaderInfoLog(shader, sizeof(log), nullptr, log);
            LOGE("Shader compile error: %s", log);
            glDeleteShader(shader);
            return 0;
        }
        return shader;
    };

    GLuint vert = compile(GL_VERTEX_SHADER, kVertexShader);
    GLuint frag = compile(GL_FRAGMENT_SHADER, kFragmentShader);
    if (!vert || !frag) return false;

    m_program = glCreateProgram();
    glAttachShader(m_program, vert);
    glAttachShader(m_program, frag);
    glLinkProgram(m_program);

    glDeleteShader(vert);
    glDeleteShader(frag);

    GLint ok;
    glGetProgramiv(m_program, GL_LINK_STATUS, &ok);
    if (!ok) {
        char log[512];
        glGetProgramInfoLog(m_program, sizeof(log), nullptr, log);
        LOGE("Program link error: %s", log);
        glDeleteProgram(m_program);
        m_program = 0;
        return false;
    }

    m_uTexture = glGetUniformLocation(m_program, "u_texture");
    m_uUVTransform = glGetUniformLocation(m_program, "u_uvTransform");

    LOGI("Timewarp shaders compiled");
    return true;
}

// --- Quaternion helpers ---

void Timewarp::quatInverse(const XrQuaternionf& q, XrQuaternionf& out) {
    // For unit quaternions, inverse = conjugate
    out.x = -q.x;
    out.y = -q.y;
    out.z = -q.z;
    out.w = q.w;
}

void Timewarp::quatMultiply(const XrQuaternionf& a, const XrQuaternionf& b, XrQuaternionf& out) {
    out.w = a.w*b.w - a.x*b.x - a.y*b.y - a.z*b.z;
    out.x = a.w*b.x + a.x*b.w + a.y*b.z - a.z*b.y;
    out.y = a.w*b.y - a.x*b.z + a.y*b.w + a.z*b.x;
    out.z = a.w*b.z + a.x*b.y - a.y*b.x + a.z*b.w;
}

void Timewarp::quatToMatrix3(const XrQuaternionf& q, float out[9]) {
    float xx = q.x*q.x, yy = q.y*q.y, zz = q.z*q.z;
    float xy = q.x*q.y, xz = q.x*q.z, yz = q.y*q.z;
    float wx = q.w*q.x, wy = q.w*q.y, wz = q.w*q.z;

    out[0] = 1 - 2*(yy+zz);  out[1] = 2*(xy-wz);      out[2] = 2*(xz+wy);
    out[3] = 2*(xy+wz);      out[4] = 1 - 2*(xx+zz);  out[5] = 2*(yz-wx);
    out[6] = 2*(xz-wy);      out[7] = 2*(yz+wx);      out[8] = 1 - 2*(xx+yy);
}
