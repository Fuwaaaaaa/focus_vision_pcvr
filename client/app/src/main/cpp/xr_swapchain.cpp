#include "xr_swapchain.h"
#include "xr_utils.h"

#include <openxr/openxr_platform.h>

void XrSwapchainWrapper::create(XrSession session, uint32_t width, uint32_t height) {
    m_width = width;
    m_height = height;

    XrSwapchainCreateInfo createInfo = {XR_TYPE_SWAPCHAIN_CREATE_INFO};
    createInfo.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT |
                            XR_SWAPCHAIN_USAGE_SAMPLED_BIT;
    createInfo.format = GL_SRGB8_ALPHA8;
    createInfo.sampleCount = 1;
    createInfo.width = width;
    createInfo.height = height;
    createInfo.faceCount = 1;
    createInfo.arraySize = 1;
    createInfo.mipCount = 1;

    XR_CHECK(xrCreateSwapchain(session, &createInfo, &m_swapchain), "xrCreateSwapchain");

    // Enumerate swapchain images
    uint32_t imageCount = 0;
    xrEnumerateSwapchainImages(m_swapchain, 0, &imageCount, nullptr);

    m_images.resize(imageCount, {XR_TYPE_SWAPCHAIN_IMAGE_OPENGL_ES_KHR});
    xrEnumerateSwapchainImages(m_swapchain, imageCount, &imageCount,
        (XrSwapchainImageBaseHeader*)m_images.data());

    // Create framebuffers for each swapchain image
    m_framebuffers.resize(imageCount);
    glGenFramebuffers(imageCount, m_framebuffers.data());

    for (uint32_t i = 0; i < imageCount; i++) {
        glBindFramebuffer(GL_FRAMEBUFFER, m_framebuffers[i]);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
            GL_TEXTURE_2D, m_images[i].image, 0);

        GLenum status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if (status != GL_FRAMEBUFFER_COMPLETE) {
            LOGW("Framebuffer[%u] incomplete: 0x%x", i, status);
        }
    }
    glBindFramebuffer(GL_FRAMEBUFFER, 0);

    LOGI("Swapchain created: %ux%u, %u images", width, height, imageCount);
}

void XrSwapchainWrapper::destroy() {
    if (!m_framebuffers.empty()) {
        glDeleteFramebuffers(m_framebuffers.size(), m_framebuffers.data());
        m_framebuffers.clear();
    }
    if (m_swapchain != XR_NULL_HANDLE) {
        xrDestroySwapchain(m_swapchain);
        m_swapchain = XR_NULL_HANDLE;
    }
    m_images.clear();
}

void XrSwapchainWrapper::acquireImage(uint32_t* outIndex) {
    XrSwapchainImageAcquireInfo acquireInfo = {XR_TYPE_SWAPCHAIN_IMAGE_ACQUIRE_INFO};
    XR_CHECK(xrAcquireSwapchainImage(m_swapchain, &acquireInfo, outIndex),
        "xrAcquireSwapchainImage");
}

void XrSwapchainWrapper::waitImage() {
    XrSwapchainImageWaitInfo waitInfo = {XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO};
    waitInfo.timeout = XR_INFINITE_DURATION;
    XR_CHECK(xrWaitSwapchainImage(m_swapchain, &waitInfo), "xrWaitSwapchainImage");
}

void XrSwapchainWrapper::releaseImage() {
    XrSwapchainImageReleaseInfo releaseInfo = {XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO};
    XR_CHECK(xrReleaseSwapchainImage(m_swapchain, &releaseInfo), "xrReleaseSwapchainImage");
}
