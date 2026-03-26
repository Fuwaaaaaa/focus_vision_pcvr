#include "renderer.h"
#include "xr_utils.h"

void Renderer::init() {
    LOGI("Renderer initialized (GLES %s)", glGetString(GL_VERSION));
    m_initialized = true;
}

void Renderer::shutdown() {
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
