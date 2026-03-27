#include "openxr_app.h"
#include "xr_utils.h"

#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>

#include <cstring>
#include <thread>

OpenXRApp::OpenXRApp() {}
OpenXRApp::~OpenXRApp() { shutdown(); }

void OpenXRApp::initialize(android_app* app) {
    LOGI("Initializing OpenXR app...");
    createInstance(app);
    getSystem();
    initEGL();
    createSession();
    createReferenceSpace();
    createSwapchains();
    m_renderer.init();
    m_timewarp.init();
    LOGI("OpenXR app initialized successfully");
}

void OpenXRApp::createInstance(android_app* app) {
    // Load OpenXR loader on Android
    PFN_xrInitializeLoaderKHR initLoader = nullptr;
    xrGetInstanceProcAddr(XR_NULL_HANDLE, "xrInitializeLoaderKHR",
        (PFN_xrVoidFunction*)&initLoader);

    if (initLoader) {
        XrLoaderInitInfoAndroidKHR loaderInit = {XR_TYPE_LOADER_INIT_INFO_ANDROID_KHR};
        loaderInit.applicationVM = app->activity->vm;
        loaderInit.applicationContext = app->activity->clazz;
        initLoader((XrLoaderInitInfoBaseHeaderKHR*)&loaderInit);
    }

    // Required extensions
    const char* extensions[] = {
        XR_KHR_OPENGL_ES_ENABLE_EXTENSION_NAME,
        XR_KHR_ANDROID_CREATE_INSTANCE_EXTENSION_NAME,
    };

    XrInstanceCreateInfoAndroidKHR androidInfo = {XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR};
    androidInfo.applicationVM = app->activity->vm;
    androidInfo.applicationActivity = app->activity->clazz;

    XrInstanceCreateInfo createInfo = {XR_TYPE_INSTANCE_CREATE_INFO};
    createInfo.next = &androidInfo;
    createInfo.enabledExtensionCount = 2;
    createInfo.enabledExtensionNames = extensions;
    strncpy(createInfo.applicationInfo.applicationName, "FocusVisionPCVR",
        XR_MAX_APPLICATION_NAME_SIZE);
    createInfo.applicationInfo.applicationVersion = 1;
    createInfo.applicationInfo.engineVersion = 1;
    strncpy(createInfo.applicationInfo.engineName, "FocusVisionEngine",
        XR_MAX_ENGINE_NAME_SIZE);
    createInfo.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;

    XR_CHECK(xrCreateInstance(&createInfo, &m_instance), "xrCreateInstance");
    LOGI("OpenXR instance created");
}

void OpenXRApp::getSystem() {
    XrSystemGetInfo systemInfo = {XR_TYPE_SYSTEM_GET_INFO};
    systemInfo.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XR_CHECK(xrGetSystem(m_instance, &systemInfo, &m_systemId), "xrGetSystem");

    // Get view configuration
    uint32_t viewCount = 0;
    xrEnumerateViewConfigurationViews(m_instance, m_systemId, m_viewConfigType,
        0, &viewCount, nullptr);
    m_viewConfigViews.resize(viewCount, {XR_TYPE_VIEW_CONFIGURATION_VIEW});
    xrEnumerateViewConfigurationViews(m_instance, m_systemId, m_viewConfigType,
        viewCount, &viewCount, m_viewConfigViews.data());

    LOGI("System: %d views, recommended %ux%u", viewCount,
        m_viewConfigViews[0].recommendedImageRectWidth,
        m_viewConfigViews[0].recommendedImageRectHeight);
}

void OpenXRApp::initEGL() {
    m_eglDisplay = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    eglInitialize(m_eglDisplay, nullptr, nullptr);

    EGLint configAttribs[] = {
        EGL_RENDERABLE_TYPE, EGL_OPENGL_ES3_BIT,
        EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
        EGL_RED_SIZE, 8,
        EGL_GREEN_SIZE, 8,
        EGL_BLUE_SIZE, 8,
        EGL_ALPHA_SIZE, 8,
        EGL_DEPTH_SIZE, 0,
        EGL_NONE
    };

    EGLint numConfigs;
    eglChooseConfig(m_eglDisplay, configAttribs, &m_eglConfig, 1, &numConfigs);

    EGLint contextAttribs[] = {EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE};
    m_eglContext = eglCreateContext(m_eglDisplay, m_eglConfig, EGL_NO_CONTEXT, contextAttribs);

    // Create a small pbuffer surface (required for making context current)
    EGLint pbufferAttribs[] = {EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE};
    m_eglSurface = eglCreatePbufferSurface(m_eglDisplay, m_eglConfig, pbufferAttribs);

    eglMakeCurrent(m_eglDisplay, m_eglSurface, m_eglSurface, m_eglContext);
    LOGI("EGL context created (GLES 3.0)");
}

void OpenXRApp::createSession() {
    XrGraphicsBindingOpenGLESAndroidKHR gfxBinding = {
        XR_TYPE_GRAPHICS_BINDING_OPENGL_ES_ANDROID_KHR};
    gfxBinding.display = m_eglDisplay;
    gfxBinding.config = m_eglConfig;
    gfxBinding.context = m_eglContext;

    XrSessionCreateInfo sessionInfo = {XR_TYPE_SESSION_CREATE_INFO};
    sessionInfo.next = &gfxBinding;
    sessionInfo.systemId = m_systemId;

    XR_CHECK(xrCreateSession(m_instance, &sessionInfo, &m_session), "xrCreateSession");
    LOGI("OpenXR session created");
}

void OpenXRApp::createReferenceSpace() {
    XrReferenceSpaceCreateInfo spaceInfo = {XR_TYPE_REFERENCE_SPACE_CREATE_INFO};
    spaceInfo.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_STAGE;
    spaceInfo.poseInReferenceSpace.orientation.w = 1.0f; // identity

    XR_CHECK(xrCreateReferenceSpace(m_session, &spaceInfo, &m_stageSpace),
        "xrCreateReferenceSpace");
    LOGI("Stage reference space created");
}

void OpenXRApp::createSwapchains() {
    for (uint32_t eye = 0; eye < 2; eye++) {
        uint32_t width = m_viewConfigViews[eye].recommendedImageRectWidth;
        uint32_t height = m_viewConfigViews[eye].recommendedImageRectHeight;
        m_swapchains[eye].create(m_session, width, height);
        LOGI("Swapchain[%u]: %ux%u", eye, width, height);
    }
}

void OpenXRApp::mainLoop() {
    m_recvBuffer.resize(2048); // Max RTP packet size

    while (m_running) {
        pollEvents();

        if (!m_sessionReady) {
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
            continue;
        }

        // Receive and decode video packets before rendering
        receiveAndDecodeVideo();
        renderFrame();
    }
}

void OpenXRApp::receiveAndDecodeVideo() {
    if (!m_networkReceiver.isInitialized()) return;

    // Drain all available UDP packets (non-blocking)
    for (int i = 0; i < 64; i++) { // Process up to 64 packets per frame
        int received = m_networkReceiver.receive(m_recvBuffer.data(), (int)m_recvBuffer.size());
        if (received <= 0) break;

        // Parse RTP header to extract FVP fields
        // RTP header: 12 bytes fixed + FVP header: 8 bytes
        if (received < 20) continue; // Too short for RTP + FVP header

        // FVP header at offset 12 (all multi-byte fields are little-endian,
        // matching Rust's to_le_bytes() in pipeline.rs):
        //   frame_index (u32 LE), shard_index (u8), total_shards (u8),
        //   flags (u16 LE)
        const uint8_t* fvp = m_recvBuffer.data() + 12;
        uint32_t frameIndex = fvp[0] | (fvp[1] << 8) | (fvp[2] << 16) | (fvp[3] << 24); // LE
        uint8_t shardIndex = fvp[4];
        uint8_t totalShards = fvp[5];
        uint16_t flags = fvp[6] | (fvp[7] << 8); // LE
        bool isKeyframe = (flags & 0x01) != 0;
        // Note: dataShards is not in the FVP header — it's derived from
        // totalShards and the FEC redundancy ratio on the receiver side.
        uint8_t dataShards = totalShards; // Approximation; FEC decoder handles actual counts

        const uint8_t* payload = m_recvBuffer.data() + 20;
        int payloadSize = received - 20;

        // New frame? Start collecting shards.
        if (frameIndex != m_fecDecoder.currentFrameIndex()) {
            // Try to decode previous frame first
            auto prevFrame = m_fecDecoder.tryDecode();
            if (prevFrame.has_value()) {
                // Validate NAL data before submitting to decoder
                const uint8_t* nalData = prevFrame->data.data();
                int nalSize = (int)prevFrame->data.size();

                // Skip Annex B start code (0x00 0x00 0x00 0x01)
                const uint8_t* nalStart = nalData;
                int nalLen = nalSize;
                if (nalSize >= 4 && nalData[0] == 0 && nalData[1] == 0 &&
                    nalData[2] == 0 && nalData[3] == 1) {
                    nalStart = nalData + 4;
                    nalLen = nalSize - 4;
                }

                auto result = NalValidator::validate(nalStart, nalLen);
                if (result == NalValidator::Result::Valid) {
                    int64_t timestampUs = prevFrame->frameIndex * 11111; // ~90fps
                    m_videoDecoder.submitPacket(nalData, nalSize, timestampUs);
                } else {
                    LOGW("NAL validation failed for frame %u, requesting IDR",
                         prevFrame->frameIndex);
                    m_tcpClient.requestIdr();
                    m_videoDecoder.flush();
                }
            }
            m_fecDecoder.beginFrame(frameIndex, totalShards, dataShards, isKeyframe);
        }

        m_fecDecoder.addShard(shardIndex, payload, payloadSize);
    }
}

void OpenXRApp::pollEvents() {
    XrEventDataBuffer event = {XR_TYPE_EVENT_DATA_BUFFER};
    while (xrPollEvent(m_instance, &event) == XR_SUCCESS) {
        switch (event.type) {
        case XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED: {
            auto* stateEvent = (XrEventDataSessionStateChanged*)&event;
            handleSessionStateChange(stateEvent->state);
            break;
        }
        case XR_TYPE_EVENT_DATA_INSTANCE_LOSS_PENDING:
            LOGW("Instance loss pending");
            m_running = false;
            break;
        default:
            break;
        }
        event = {XR_TYPE_EVENT_DATA_BUFFER};
    }
}

void OpenXRApp::handleSessionStateChange(XrSessionState newState) {
    m_sessionState = newState;
    LOGI("Session state: %d", (int)newState);

    switch (newState) {
    case XR_SESSION_STATE_READY: {
        XrSessionBeginInfo beginInfo = {XR_TYPE_SESSION_BEGIN_INFO};
        beginInfo.primaryViewConfigurationType = m_viewConfigType;
        XR_CHECK(xrBeginSession(m_session, &beginInfo), "xrBeginSession");
        m_sessionReady = true;
        LOGI("Session started");
        break;
    }
    case XR_SESSION_STATE_STOPPING:
        XR_CHECK(xrEndSession(m_session), "xrEndSession");
        m_sessionReady = false;
        LOGI("Session stopped");
        break;
    case XR_SESSION_STATE_EXITING:
    case XR_SESSION_STATE_LOSS_PENDING:
        m_running = false;
        break;
    default:
        break;
    }
}

void OpenXRApp::renderFrame() {
    XrFrameWaitInfo waitInfo = {XR_TYPE_FRAME_WAIT_INFO};
    XrFrameState frameState = {XR_TYPE_FRAME_STATE};
    XR_CHECK(xrWaitFrame(m_session, &waitInfo, &frameState), "xrWaitFrame");

    XrFrameBeginInfo beginInfo = {XR_TYPE_FRAME_BEGIN_INFO};
    XR_CHECK(xrBeginFrame(m_session, &beginInfo), "xrBeginFrame");

    std::vector<XrCompositionLayerBaseHeader*> layers;
    XrCompositionLayerProjection projLayer = {XR_TYPE_COMPOSITION_LAYER_PROJECTION};
    std::array<XrCompositionLayerProjectionView, 2> projViews;

    if (frameState.shouldRender == XR_TRUE) {
        // Locate views (eye poses + projection)
        XrViewLocateInfo locateInfo = {XR_TYPE_VIEW_LOCATE_INFO};
        locateInfo.viewConfigurationType = m_viewConfigType;
        locateInfo.displayTime = frameState.predictedDisplayTime;
        locateInfo.space = m_stageSpace;

        XrViewState viewState = {XR_TYPE_VIEW_STATE};
        uint32_t viewCount = 2;
        std::array<XrView, 2> views;
        views[0] = {XR_TYPE_VIEW};
        views[1] = {XR_TYPE_VIEW};

        xrLocateViews(m_session, &locateInfo, &viewState, 2, &viewCount, views.data());

        // Render each eye
        for (uint32_t eye = 0; eye < 2; eye++) {
            uint32_t imgIndex;
            m_swapchains[eye].acquireImage(&imgIndex);
            m_swapchains[eye].waitImage();

            GLuint framebuffer = m_swapchains[eye].getFramebuffer(imgIndex);
            uint32_t width = m_swapchains[eye].getWidth();
            uint32_t height = m_swapchains[eye].getHeight();

            // Rendering decision: new frame or timewarp
            bool hasNewFrame = m_videoDecoder.getDecodedFrame();

            if (hasNewFrame) {
                m_lastDecodedTexture = m_videoDecoder.getOutputTexture();
            }

            if (hasNewFrame && m_lastDecodedTexture != 0) {
                // Normal path: render the new decoded video frame
                m_poseHistory.record(m_lastFrameIndex, views[eye].pose,
                    frameState.predictedDisplayTime);
                m_renderer.renderVideoFrame(framebuffer, width, height,
                    m_lastDecodedTexture);
                m_hasDecodedFrame = true;
                m_lastFrameIndex++;
            } else if (m_hasDecodedFrame && m_lastDecodedTexture != 0) {
                // Timewarp path: re-render previous frame with rotation correction
                auto record = m_poseHistory.latest();
                if (record.has_value()) {
                    m_timewarp.apply(framebuffer, width, height,
                        m_lastDecodedTexture,
                        record->pose,       // pose when frame was rendered
                        views[eye].pose,     // current predicted pose
                        views[eye].fov);
                } else {
                    m_renderer.renderSolidColor(framebuffer, width, height,
                        0.05f, 0.05f, 0.2f);
                }
            } else {
                // No frame yet: solid color
                m_renderer.renderSolidColor(framebuffer, width, height,
                    0.05f, 0.05f, 0.2f);
            }

            m_swapchains[eye].releaseImage();

            // Setup projection view
            projViews[eye] = {XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW};
            projViews[eye].pose = views[eye].pose;
            projViews[eye].fov = views[eye].fov;
            projViews[eye].subImage.swapchain = m_swapchains[eye].getHandle();
            projViews[eye].subImage.imageRect.offset = {0, 0};
            projViews[eye].subImage.imageRect.extent = {
                (int32_t)width, (int32_t)height};
            projViews[eye].subImage.imageArrayIndex = 0;
        }

        projLayer.space = m_stageSpace;
        projLayer.viewCount = 2;
        projLayer.views = projViews.data();
        layers.push_back((XrCompositionLayerBaseHeader*)&projLayer);
    }

    XrFrameEndInfo endInfo = {XR_TYPE_FRAME_END_INFO};
    endInfo.displayTime = frameState.predictedDisplayTime;
    endInfo.environmentBlendMode = XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
    endInfo.layerCount = (uint32_t)layers.size();
    endInfo.layers = layers.data();

    XR_CHECK(xrEndFrame(m_session, &endInfo), "xrEndFrame");
}

void OpenXRApp::pollAndroidEvents(android_app* app) {
    int events;
    struct android_poll_source* source;
    while (ALooper_pollAll(0, nullptr, &events, (void**)&source) >= 0) {
        if (source) source->process(app, source);
        if (app->destroyRequested) {
            m_running = false;
            return;
        }
    }
}

void OpenXRApp::shutdown() {
    if (m_stageSpace != XR_NULL_HANDLE) {
        xrDestroySpace(m_stageSpace);
        m_stageSpace = XR_NULL_HANDLE;
    }
    for (auto& sc : m_swapchains) sc.destroy();
    if (m_session != XR_NULL_HANDLE) {
        xrDestroySession(m_session);
        m_session = XR_NULL_HANDLE;
    }
    if (m_instance != XR_NULL_HANDLE) {
        xrDestroyInstance(m_instance);
        m_instance = XR_NULL_HANDLE;
    }
    if (m_eglDisplay != EGL_NO_DISPLAY) {
        eglMakeCurrent(m_eglDisplay, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
        if (m_eglSurface != EGL_NO_SURFACE) eglDestroySurface(m_eglDisplay, m_eglSurface);
        if (m_eglContext != EGL_NO_CONTEXT) eglDestroyContext(m_eglDisplay, m_eglContext);
        eglTerminate(m_eglDisplay);
    }
    LOGI("OpenXR app shut down");
}
