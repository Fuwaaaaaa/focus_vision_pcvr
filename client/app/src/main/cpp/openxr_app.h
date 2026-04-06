#pragma once

#include "platform_defines.h"
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <android_native_app_glue.h>

#include "xr_swapchain.h"
#include "renderer.h"
#include "network_receiver.h"
#include "video_decoder.h"
#include "fec_decoder.h"
#include "nal_validator.h"
#include "timewarp.h"
#include "overlay_renderer.h"
#include "pose_history.h"
#include "tracking_sender.h"
#include "controller_poller.h"
#include "tcp_client.h"
#include "audio_player.h"
#include "eye_tracker.h"
#include "hmd_profile.h"
#include "heartbeat_client.h"
#include "stats_reporter.h"

#include <vector>
#include <array>
#include <string>

/// Pairing flow states for HMD UI overlay
enum class PairingState {
    Idle,           // Not started
    Searching,      // Looking for PC server (TCP connect)
    PinEntry,       // Waiting for user to enter PIN
    Verifying,      // PIN submitted, waiting for result
    Failed,         // Wrong PIN (shows remaining attempts)
    LockedOut,      // Too many failures (shows countdown)
    Connected,      // Successfully paired, streaming active
    Disconnected,   // Was connected, lost connection
};

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
    void receiveAndDecodeVideo();

    void handleSessionStateChange(XrSessionState newState);

    // Android app reference (for JNI access)
    android_app* m_androidApp = nullptr;

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

    // Network + decode pipeline
    NetworkReceiver m_networkReceiver;
    FecFrameDecoder m_fecDecoder;
    VideoDecoder m_videoDecoder;
    TcpControlClient m_tcpClient;

    // Video receive buffer
    std::vector<uint8_t> m_recvBuffer;

    // Audio
    AudioPlayer m_audioPlayer;
    NetworkReceiver m_audioReceiver;

    // Tracking + controllers + eye tracking
    TrackingSender m_trackingSender;
    ControllerPoller m_controllerPoller;
    EyeTracker m_eyeTracker;
    HmdProfile m_hmdProfile;

    // Timewarp
    Timewarp m_timewarp;
    OverlayRenderer m_overlay;
    PoseHistory m_poseHistory;

    // Heartbeat + stats
    HeartbeatClient m_heartbeat;
    StatsReporter m_stats;

    // State: last decoded frame
    GLuint m_lastDecodedTexture = 0;
    uint32_t m_lastFrameIndex = 0;
    bool m_hasDecodedFrame = false;

    // Connection and pairing state
    PairingState m_pairingState = PairingState::Idle;
    uint8_t m_pinAttemptsRemaining = 3;
    std::string m_pairingMessage;
    bool m_streamingActive = false;
    std::chrono::steady_clock::time_point m_lastPacketTime;
    static constexpr int DISCONNECT_TIMEOUT_MS = 2000; // 2s without packets = disconnected
};
