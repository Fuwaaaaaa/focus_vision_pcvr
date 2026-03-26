#pragma once

#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <EGL/egl.h>
#include <GLES3/gl3.h>
#include <android_native_app_glue.h>

#include "xr_swapchain.h"
#include "renderer.h"
#include "network_receiver.h"
#include "video_decoder.h"
#include "timewarp.h"
#include "pose_history.h"
#include "tracking_sender.h"
#include "controller_poller.h"
#include "tcp_client.h"

#include <vector>
#include <array>

class OpenXRApp {
public:
    OpenXRApp();
    ~OpenXRApp();

    void initialize(android_app* app);
    void mainLoop();
    void shutdown();

    bool isRunning() const { return m_running; }

private:
    void createInstance(android_app* app);
    void getSystem();
    void initEGL();
    void createSession();
    void createReferenceSpace();
    void createSwapchains();
    void pollEvents();
    void pollAndroidEvents(android_app* app);
    void renderFrame();

    void handleSessionStateChange(XrSessionState newState);

    // OpenXR handles
    XrInstance m_instance = XR_NULL_HANDLE;
    XrSystemId m_systemId = XR_NULL_SYSTEM_ID;
    XrSession m_session = XR_NULL_HANDLE;
    XrSpace m_stageSpace = XR_NULL_HANDLE;

    // Session state
    XrSessionState m_sessionState = XR_SESSION_STATE_UNKNOWN;
    bool m_running = true;
    bool m_sessionReady = false;

    // EGL context
    EGLDisplay m_eglDisplay = EGL_NO_DISPLAY;
    EGLContext m_eglContext = EGL_NO_CONTEXT;
    EGLSurface m_eglSurface = EGL_NO_SURFACE;
    EGLConfig m_eglConfig = nullptr;

    // Per-eye swapchains
    std::array<XrSwapchainWrapper, 2> m_swapchains;

    // View configuration
    XrViewConfigurationType m_viewConfigType = XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
    std::vector<XrViewConfigurationView> m_viewConfigViews;

    // Renderer
    Renderer m_renderer;

    // Network + decode
    NetworkReceiver m_networkReceiver;
    VideoDecoder m_videoDecoder;
    TcpControlClient m_tcpClient;

    // Tracking + controllers
    TrackingSender m_trackingSender;
    ControllerPoller m_controllerPoller;

    // Timewarp
    Timewarp m_timewarp;
    PoseHistory m_poseHistory;

    // State: last decoded frame
    GLuint m_lastDecodedTexture = 0;
    uint32_t m_lastFrameIndex = 0;
    bool m_hasDecodedFrame = false;
};
