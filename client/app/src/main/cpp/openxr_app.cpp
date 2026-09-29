#include "openxr_app.h"
#include "xr_utils.h"
#include "client_protocol.h"

#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <iterator>
#include <sstream>
#include <thread>

/// Read HMD battery level from Android sysfs (0-100, or -1 on failure).
static int readBatteryLevel() {
    FILE* f = fopen("/sys/class/power_supply/battery/capacity", "r");
    if (!f) return -1;
    int level = -1;
    if (fscanf(f, "%d", &level) != 1) level = -1;
    fclose(f);
    return level;
}

OpenXRApp::OpenXRApp() {}
OpenXRApp::~OpenXRApp() { shutdown(); }

void OpenXRApp::initialize(android_app* app) {
    LOGI("Initializing OpenXR app...");
    m_androidApp = app;
    createInstance(app);
    getSystem();
    if (!initEGL()) {
        LOGE("EGL initialization failed — aborting OpenXR session creation to "
             "avoid crashing in createSession() with EGL_NO_CONTEXT");
        m_running = false;
        return;
    }
    createSession();
    createReferenceSpace();
    createSwapchains();
    initInput();
    m_renderer.init();
    m_overlay.init();
    if (m_extensions.facialTracking) m_facialTracker.init(m_instance, m_session);

    // App-private storage (uninstalling or clearing the app's data removes
    // it): the TOFU pin of the paired PC, and the launch request with the
    // PC's address and PIN that MainActivity writes.
    if (app->activity && app->activity->internalDataPath) {
        const std::string dataDir = app->activity->internalDataPath;
        m_fingerprintPath = dataDir + "/server_fingerprint.hex";
        m_launchRequestPath = dataDir + "/" + fvp_launch::LAUNCH_REQUEST_FILE;
    } else {
        LOGE("internalDataPath unavailable — pairing will refuse to connect");
    }

    // The decoder starts at the native size, both eyes side by side;
    // STREAM_CONFIG may change the codec or size (resolution_scale) when a
    // session starts.
    TcpControlClient::StreamConfig native;
    native.width = native.encodedWidth = 1832;
    native.height = native.encodedHeight = 1920;
    native.codec = 1;
    native.layout = fvp_client_protocol::STEREO_SIDE_BY_SIDE;  // what a v5 server sends
    configureDecoder(native);

    // Video is received on its own thread, bound once; each session points
    // it at the server before STREAM_START so the first packets are kept.
    m_videoReceiver.start(m_udpBasePort + fvp_client_protocol::VIDEO_PORT_OFFSET,
                          &m_stream.stats(), [this] { m_stream.requestIdr(); });
    m_stream.setStreamStartHook(
        [this](const TcpControlClient::StreamConfig&, const std::string& serverIp) {
            m_videoReceiver.beginSession(serverIp);
        });
    m_stream.setStreamEndHook([this] { m_videoReceiver.endSession(); });

    checkLaunchRequest();
    LOGI("OpenXR app initialized successfully");
}

void OpenXRApp::withJni(const std::function<void(JNIEnv*)>& fn) {
    // Detach only if attached here: an unbalanced attach leaks a VM
    // reference and logs a warning at exit on some Android versions.
    JavaVM* vm = m_androidApp->activity->vm;
    JNIEnv* env = nullptr;
    bool didAttach = false;
    if (vm->GetEnv(reinterpret_cast<void**>(&env), JNI_VERSION_1_6) == JNI_EDETACHED) {
        if (vm->AttachCurrentThread(&env, nullptr) != JNI_OK) {
            LOGE("AttachCurrentThread failed");
            env = nullptr;
        } else {
            didAttach = true;
        }
    }
    fn(env);
    if (didAttach) vm->DetachCurrentThread();
}

void OpenXRApp::configureDecoder(const TcpControlClient::StreamConfig& config) {
    auto dims = fvp_client_protocol::decoderInitDims(
        config.width, config.height, config.encodedWidth, config.encodedHeight, config.layout);
    if (dims.width == 0 || dims.height == 0) dims = {1832, 1920};
    const uint8_t codec = config.codec == 0 ? 0 : 1;
    m_videoLayout = config.layout;
    m_framePose = config.framePose;
    m_renderPoses.clear();
    if (m_videoDecoder.isInitialized() && codec == m_decoderCodec &&
        dims.width == m_decoderWidth && dims.height == m_decoderHeight) {
        m_videoDecoder.flush(); // a new session starts from a keyframe
        return;
    }
    // Needs the EGL context, current on this (render) thread since initEGL.
    // A null JNIEnv makes the decoder fall back to buffer output.
    const char* mime = codec == 0 ? "video/avc" : "video/hevc";
    withJni([&](JNIEnv* env) {
        m_videoDecoder.shutdown();
        m_hasDecodedFrame = false;
        m_lastDecodedTexture = 0;
        if (m_videoDecoder.init(env, static_cast<int>(dims.width), static_cast<int>(dims.height), mime)) {
            m_decoderCodec = codec;
            m_decoderWidth = dims.width;
            m_decoderHeight = dims.height;
        }
    });
}

void OpenXRApp::checkLaunchRequest() {
    m_lastLaunchCheck = std::chrono::steady_clock::now();
    if (m_launchRequestPath.empty()) return;
    std::ifstream in(m_launchRequestPath);
    if (!in.good()) return;
    std::stringstream text;
    text << in.rdbuf();
    in.close();
    // Used once: a stale PIN would cost an attempt against the engine's lockout.
    std::remove(m_launchRequestPath.c_str());

    fvp_launch::LaunchRequest request;
    if (!fvp_launch::parseLaunchRequest(text.str(), request)) {
        LOGE("Ignoring a malformed launch request");
        return;
    }
    startStreamSession(request);
}

void OpenXRApp::startStreamSession(const fvp_launch::LaunchRequest& request) {
    LOGI("Connecting to %s:%u (UDP base %u)", request.server.ip.c_str(), request.server.port,
         request.udpBasePort);
    m_stream.stop(); // leaves a previous PC with DISCONNECT
    m_trackingSender.shutdown();
    if (request.udpBasePort != m_udpBasePort) {
        m_udpBasePort = request.udpBasePort;
        m_videoReceiver.start(m_udpBasePort + fvp_client_protocol::VIDEO_PORT_OFFSET,
                              &m_stream.stats(), [this] { m_stream.requestIdr(); });
        m_audioReceiver.shutdown();
    }
    m_serverIp = request.server.ip;

    SessionSettings settings;
    settings.serverIp = request.server.ip;
    settings.controlPort = request.server.port;
    settings.pin = request.pin;
    settings.fingerprintStorePath = m_fingerprintPath;
    m_pairingState = PairingState::Searching;
    m_stream.start(settings);
}

void OpenXRApp::handleStreamEvents() {
    ServerEvent e;
    while (m_stream.pollEvent(e)) {
        switch (e.type) {
        case ServerEvent::Type::Streaming: {
            const auto config = m_stream.streamConfig();
            configureDecoder(config);
            m_frameDurationUs = config.framerate > 0 ? 1000000 / config.framerate : 11111;
            m_dashboardBitrate = config.bitrateMbps;
            m_dashboardCodecH265 = config.codec != 0;
            m_trackingSender.shutdown();
            m_trackingSender.init(m_serverIp.c_str(),
                                  m_udpBasePort + fvp_client_protocol::TRACKING_PORT_OFFSET);
            if (!m_audioReceiver.isInitialized()) {
                m_audioReceiver.init("0.0.0.0", m_udpBasePort + fvp_client_protocol::AUDIO_PORT_OFFSET);
            }
            if (!m_audioPlayer.isInitialized()) m_audioPlayer.init();
            m_sleeping = false;
            m_pairingState = PairingState::Connected;
            LOGI("Streaming from %s", m_serverIp.c_str());
            break;
        }
        case ServerEvent::Type::Disconnected:
            // StreamSession reconnects on its own; the engine takes the same
            // PIN for 5 s after a drop.
            m_trackingSender.shutdown();
            m_pairingState = PairingState::Disconnected;
            break;
        case ServerEvent::Type::PinRejected:
            m_pairingState = PairingState::Failed;
            m_pairingMessage = "PIN rejected: send a new one from the companion app";
            LOGE("%s", m_pairingMessage.c_str());
            break;
        case ServerEvent::Type::Haptic:
            // Does nothing if the controller actions failed (initInput).
            m_controllerPoller.applyHaptic(m_session, e.haptic.controllerId,
                                           e.haptic.durationMs / 1000.0f,
                                           e.haptic.frequency, e.haptic.amplitude);
            break;
        case ServerEvent::Type::SleepEnter:
            m_sleeping = true;
            break;
        case ServerEvent::Type::SleepExit:
            m_sleeping = false;
            break;
        case ServerEvent::Type::HeartbeatAck:
            m_pcLatency = e.heartbeatAck;
            break;
        case ServerEvent::Type::ConfigUpdateAck:
            LOGI("CONFIG_UPDATE 0x%02x %s", e.configKey, e.configAccepted ? "accepted" : "rejected");
            break;
        }
    }
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

    // The two required extensions, and each optional one the runtime offers
    // (Focus 3 controller profile, eye gaze, facial tracking): their
    // profiles and functions exist only if enabled here.
    uint32_t extCount = 0;
    xrEnumerateInstanceExtensionProperties(nullptr, 0, &extCount, nullptr);
    std::vector<XrExtensionProperties> props(extCount, {XR_TYPE_EXTENSION_PROPERTIES});
    xrEnumerateInstanceExtensionProperties(nullptr, extCount, &extCount, props.data());
    std::vector<std::string> available;
    for (const auto& p : props) available.emplace_back(p.extensionName);
    m_extensions = fvp_xr::chooseExtensions(available);
    if (m_extensions.missingRequired) {
        LOGE("OpenXR runtime lacks a required extension (OpenGL ES / Android instance)");
    }
    LOGI("OpenXR extensions: Focus 3 controller %s, eye gaze %s, facial tracking %s",
        m_extensions.focus3Controller ? "yes" : "no", m_extensions.eyeGaze ? "yes" : "no",
        m_extensions.facialTracking ? "yes" : "no");

    XrInstanceCreateInfoAndroidKHR androidInfo = {XR_TYPE_INSTANCE_CREATE_INFO_ANDROID_KHR};
    androidInfo.applicationVM = app->activity->vm;
    androidInfo.applicationActivity = app->activity->clazz;

    XrInstanceCreateInfo createInfo = {XR_TYPE_INSTANCE_CREATE_INFO};
    createInfo.next = &androidInfo;
    createInfo.enabledExtensionCount = static_cast<uint32_t>(m_extensions.names.size());
    createInfo.enabledExtensionNames = m_extensions.names.data();
    strncpy(createInfo.applicationInfo.applicationName, "FocusVisionPCVR",
        XR_MAX_APPLICATION_NAME_SIZE);
    createInfo.applicationInfo.applicationVersion = 1;
    createInfo.applicationInfo.engineVersion = 1;
    strncpy(createInfo.applicationInfo.engineName, "FocusVisionEngine",
        XR_MAX_ENGINE_NAME_SIZE);
    // 1.0: the app uses nothing from 1.1, and a 1.0-only runtime refuses a
    // 1.1 request with XR_ERROR_API_VERSION_UNSUPPORTED.
    createInfo.applicationInfo.apiVersion = XR_API_VERSION_1_0;

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

bool OpenXRApp::initEGL() {
    m_eglDisplay = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if (m_eglDisplay == EGL_NO_DISPLAY) {
        LOGE("eglGetDisplay failed");
        return false;
    }
    if (!eglInitialize(m_eglDisplay, nullptr, nullptr)) {
        LOGE("eglInitialize failed: 0x%x", eglGetError());
        return false;
    }

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
    if (!eglChooseConfig(m_eglDisplay, configAttribs, &m_eglConfig, 1, &numConfigs) || numConfigs == 0) {
        LOGE("eglChooseConfig failed: 0x%x", eglGetError());
        return false;
    }

    EGLint contextAttribs[] = {EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE};
    m_eglContext = eglCreateContext(m_eglDisplay, m_eglConfig, EGL_NO_CONTEXT, contextAttribs);
    if (m_eglContext == EGL_NO_CONTEXT) {
        LOGE("eglCreateContext failed: 0x%x", eglGetError());
        return false;
    }

    // Create a small pbuffer surface (required for making context current)
    EGLint pbufferAttribs[] = {EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE};
    m_eglSurface = eglCreatePbufferSurface(m_eglDisplay, m_eglConfig, pbufferAttribs);
    if (m_eglSurface == EGL_NO_SURFACE) {
        LOGE("eglCreatePbufferSurface failed: 0x%x", eglGetError());
        return false;
    }

    if (!eglMakeCurrent(m_eglDisplay, m_eglSurface, m_eglSurface, m_eglContext)) {
        LOGE("eglMakeCurrent failed: 0x%x", eglGetError());
        return false;
    }
    LOGI("EGL context created (GLES 3.0)");
    return true;
}

void OpenXRApp::createSession() {
    // The spec requires this call before xrCreateSession, which otherwise
    // fails with XR_ERROR_GRAPHICS_REQUIREMENTS_CALL_MISSING.
    PFN_xrGetOpenGLESGraphicsRequirementsKHR getGraphicsRequirements = nullptr;
    XR_CHECK(xrGetInstanceProcAddr(m_instance, "xrGetOpenGLESGraphicsRequirementsKHR",
        reinterpret_cast<PFN_xrVoidFunction*>(&getGraphicsRequirements)),
        "xrGetInstanceProcAddr(xrGetOpenGLESGraphicsRequirementsKHR)");
    XrGraphicsRequirementsOpenGLESKHR requirements = {XR_TYPE_GRAPHICS_REQUIREMENTS_OPENGL_ES_KHR};
    XR_CHECK(getGraphicsRequirements(m_instance, m_systemId, &requirements),
        "xrGetOpenGLESGraphicsRequirementsKHR");

    GLint major = 0, minor = 0;
    glGetIntegerv(GL_MAJOR_VERSION, &major);
    glGetIntegerv(GL_MINOR_VERSION, &minor);
    const XrVersion contextVersion = XR_MAKE_VERSION(major, minor, 0);
    LOGI("GLES %d.%d; runtime supports %u.%u to %u.%u", major, minor,
        XR_VERSION_MAJOR(requirements.minApiVersionSupported),
        XR_VERSION_MINOR(requirements.minApiVersionSupported),
        XR_VERSION_MAJOR(requirements.maxApiVersionSupported),
        XR_VERSION_MINOR(requirements.maxApiVersionSupported));
    if (contextVersion < requirements.minApiVersionSupported) {
        throw std::runtime_error("GLES context is older than the OpenXR runtime requires");
    }

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

    // The head, where the eye gaze is located.
    spaceInfo.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_VIEW;
    XR_CHECK(xrCreateReferenceSpace(m_session, &spaceInfo, &m_viewSpace),
        "xrCreateReferenceSpace (view)");
}

void OpenXRApp::initInput() {
    // REGRESSION: neither the controllers nor the eye tracker were ever
    // initialized, and each attached its own action set — a session allows
    // one xrAttachSessionActionSets, so the second always failed.
    m_actionSets.clear();
    if (XrActionSet set = m_controllerPoller.createActions(m_instance, m_extensions.focus3Controller)) {
        m_actionSets.push_back(set);
    }
    if (XrActionSet set = m_eyeTracker.createActions(m_instance, m_systemId, m_extensions.eyeGaze)) {
        m_actionSets.push_back(set);
    }
    if (m_actionSets.empty()) {
        LOGE("No input action sets: controllers are not tracked");
        return;
    }

    XrSessionActionSetsAttachInfo attachInfo = {XR_TYPE_SESSION_ACTION_SETS_ATTACH_INFO};
    attachInfo.actionSets = m_actionSets.data();
    attachInfo.countActionSets = static_cast<uint32_t>(m_actionSets.size());
    if (XR_FAILED(xrAttachSessionActionSets(m_session, &attachInfo))) {
        LOGE("xrAttachSessionActionSets failed: controllers and eye gaze are unavailable");
        m_actionSets.clear();
        return;
    }
    m_controllerPoller.createSpaces(m_session);
    m_eyeTracker.createSpace(m_session, m_viewSpace);
}

void OpenXRApp::syncActions() {
    if (m_actionSets.empty()) return;
    std::vector<XrActiveActionSet> active;
    for (XrActionSet set : m_actionSets) active.push_back({set, XR_NULL_PATH});
    XrActionsSyncInfo syncInfo = {XR_TYPE_ACTIONS_SYNC_INFO};
    syncInfo.activeActionSets = active.data();
    syncInfo.countActiveActionSets = static_cast<uint32_t>(active.size());
    xrSyncActions(m_session, &syncInfo);
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
    uint32_t frameCount = 0;

    while (m_running) {
        // This loop only returns when the app stops, so Android lifecycle
        // commands (pause, resume, destroy) must be handled here; left
        // unread, the activity's onPause blocks and the system reports ANR.
        pollAndroidEvents(m_androidApp);
        if (!m_running) break;
        pollEvents();

        // A new launch from the companion (onNewIntent) replaces the connection.
        if (std::chrono::steady_clock::now() - m_lastLaunchCheck > std::chrono::seconds(1)) {
            checkLaunchRequest();
        }
        handleStreamEvents();

        if (!m_sessionReady) {
            // Not rendering: keep the frame queue from filling up. Decoding
            // resumes at a keyframe (requireKeyframe below).
            FrameAssembler::Frame skipped;
            while (m_videoReceiver.popFrame(skipped)) {}
            m_rendering = false;
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
            continue;
        }
        if (!m_rendering) {
            m_videoReceiver.requireKeyframe();
            m_rendering = true;
        }

        // Update HMD battery level every ~30 seconds (2700 frames at 90fps)
        if (frameCount % 2700 == 0) {
            int level = readBatteryLevel();
            if (level >= 0 && level <= 100) {
                m_controllerPoller.setHmdBattery(static_cast<uint8_t>(level));
            }
        }

        // Feed the decoder before rendering
        receiveAndDecodeVideo();
        receiveAudio();
        renderFrame();
        frameCount++;
    }
}

void OpenXRApp::receiveAudio() {
    if (!m_audioReceiver.isInitialized() || !m_audioPlayer.isInitialized()) return;
    // Opus over RTP from the engine: a 12-byte RTP header, then one 10 ms
    // Opus packet. A few arrive per rendered frame.
    constexpr int kRtpHeader = 12;
    m_audioBuffer.resize(2048);
    for (;;) {
        const int n = m_audioReceiver.receive(m_audioBuffer.data(), static_cast<int>(m_audioBuffer.size()));
        if (n <= 0) break;
        if (n > kRtpHeader) {
            m_audioPlayer.submitOpusPacket(m_audioBuffer.data() + kRtpHeader, n - kRtpHeader);
        }
    }
    m_audioPlayer.pump();
}

void OpenXRApp::receiveAndDecodeVideo() {
    // VideoReceiver's thread receives the packets and rebuilds the frames;
    // they arrive complete, in order, starting at a keyframe.
    FrameAssembler::Frame frame;
    while (m_videoReceiver.popFrame(frame)) {
        submitDecodedFrame(frame.data.data(), static_cast<int>(frame.data.size()), frame.frameIndex);
    }
}

void OpenXRApp::submitDecodedFrame(const uint8_t* nalData, int nalSize, uint32_t frameIndex) {
    // v6: the orientation the frame was rendered at comes first; kept until
    // the decoder hands the frame back.
    if (m_framePose) {
        fvp_client_protocol::FramePose pose;
        const size_t prefix = fvp_client_protocol::parseFramePose(nalData, static_cast<size_t>(nalSize), pose);
        if (prefix == 0) {
            LOGW("Frame %u has no render pose, waiting for a keyframe", frameIndex);
            m_videoReceiver.requireKeyframe();
            return;
        }
        m_renderPoses.record(frameIndex, pose);
        nalData += prefix;
        nalSize -= static_cast<int>(prefix);
    }

    // Skip Annex B start code: 4-byte (00 00 00 01) or 3-byte (00 00 01)
    const uint8_t* nalStart = nalData;
    int nalLen = nalSize;
    if (nalSize >= 4 && nalData[0] == 0 && nalData[1] == 0 &&
        nalData[2] == 0 && nalData[3] == 1) {
        nalStart = nalData + 4;
        nalLen = nalSize - 4;
    } else if (nalSize >= 3 && nalData[0] == 0 && nalData[1] == 0 && nalData[2] == 1) {
        nalStart = nalData + 3;
        nalLen = nalSize - 3;
    }

    // NalValidator knows the HEVC NAL header; an H.264 header has a
    // different layout, so only its forbidden_zero_bit is checked.
    const bool valid = m_decoderCodec == 1
        ? NalValidator::validate(nalStart, nalLen) == NalValidator::Result::Valid
        : nalLen >= 1 && (nalStart[0] & 0x80) == 0;
    if (!valid) {
        LOGW("NAL validation failed for frame %u, waiting for a keyframe", frameIndex);
        m_videoDecoder.flush();
        m_videoReceiver.requireKeyframe();
        return;
    }
    const int64_t timestampUs = static_cast<int64_t>(frameIndex) * m_frameDurationUs;
    if (!m_videoDecoder.submitPacket(nalData, nalSize, timestampUs)) {
        // No decoder input buffer: this frame is lost, and so are the ones
        // that reference it.
        LOGW("Decoder input full — frame %u dropped, waiting for a keyframe", frameIndex);
        m_videoReceiver.requireKeyframe();
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

        XR_CHECK(xrLocateViews(m_session, &locateInfo, &viewState, 2, &viewCount, views.data()),
                 "xrLocateViews");

        // The PC renders SteamVR's views with the same fields of view and
        // IPD (VIEW_CONFIG). The session sends it once per connection and
        // when it changes (the IPD dial), not every frame.
        if (viewCount == 2 && (viewState.viewStateFlags & XR_VIEW_STATE_POSITION_VALID_BIT)) {
            auto eyeFov = [](const XrFovf& f) {
                return fvp_client_protocol::EyeFov{f.angleLeft, f.angleRight, f.angleUp, f.angleDown};
            };
            const XrVector3f& a = views[0].pose.position;
            const XrVector3f& b = views[1].pose.position;
            const float ipd = std::sqrt((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y) +
                                        (a.z - b.z) * (a.z - b.z));
            m_stream.setViewConfig(fvp_client_protocol::buildViewConfigPayload(
                eyeFov(views[0].fov), eyeFov(views[1].fov), ipd));
        }

        // One sync for every action set, then read them.
        syncActions();

        // Poll eye gaze and send head tracking + gaze data to PC. The gaze
        // is placed in the left eye's image (the foveated QP map puts it in
        // the same place in both eyes).
        const XrFovf& gazeFov = views[0].fov;
        auto gaze = m_eyeTracker.poll(frameState.predictedDisplayTime,
            fvp_video::tangents(gazeFov.angleLeft, gazeFov.angleRight, gazeFov.angleDown, gazeFov.angleUp));
        m_trackingSender.sendHeadPose(views[0].pose, frameState.predictedDisplayTime,
                                       gaze.x, gaze.y, gaze.valid);

        // Poll controllers and send state to PC
        m_controllerPoller.pollAndSend(m_session, m_stageSpace,
            frameState.predictedDisplayTime, m_trackingSender);

        // Poll face tracking and send blendshapes to PC via TCP (msg 0x35)
        if (m_facialTracker.isAvailable() && m_stream.isStreaming()) {
            auto face = m_facialTracker.poll();
            if (face.lipValid || face.eyeValid) {
                // Pack: [lip_valid:1B][eye_valid:1B][lip:37*4B][eye:14*4B] = 206 bytes
                uint8_t buf[206];
                int off = 0;
                buf[off++] = face.lipValid ? 1 : 0;
                buf[off++] = face.eyeValid ? 1 : 0;
                memcpy(buf + off, face.lip.data(), 37 * 4); off += 37 * 4;
                memcpy(buf + off, face.eye.data(), 14 * 4); off += 14 * 4;
                m_stream.send(fvp_client_protocol::msg::FACE_DATA, buf, static_cast<size_t>(off));
            }
        }

        // Connection loss: a stream was shown, and now the session is down
        // or no video packet came for DISCONNECT_TIMEOUT_MS.
        const bool streaming = m_stream.isStreaming();
        const bool connectionLost = m_hasDecodedFrame &&
            (!streaming ||
             std::chrono::duration_cast<std::chrono::milliseconds>(
                 std::chrono::steady_clock::now() - m_videoReceiver.lastPacketTime()).count()
                 > DISCONNECT_TIMEOUT_MS);

        // The newest decoded frame, taken once: both eyes show the same
        // frame (each its own half). It used to be taken per eye, so the
        // eyes could show different frames.
        bool hasNewFrame = false;
        if (!connectionLost && m_videoDecoder.getDecodedFrame()) {
            const GLuint texture = m_videoDecoder.getOutputTexture();
            m_stream.stats().onFrameDecoded(m_videoDecoder.lastDecodeLatencyUs());
            if (texture != 0) {
                hasNewFrame = true;
                m_lastDecodedTexture = texture;
                m_hasDecodedFrame = true;
                // Turned from the orientation it was rendered at (v6), or,
                // not knowing that, from where the head is now: later loops
                // reproject it from there.
                float rendered[4];
                const uint32_t shown = RenderPoseLog::frameIndexOf(
                    m_videoDecoder.lastPresentationTimeUs(), m_frameDurationUs);
                m_frameHasRenderPose = m_framePose && m_renderPoses.find(shown, rendered);
                m_frameOrientation = m_frameHasRenderPose
                    ? XrQuaternionf{rendered[0], rendered[1], rendered[2], rendered[3]}
                    : views[0].pose.orientation;
            }
        }

        // Render each eye
        for (uint32_t eye = 0; eye < 2; eye++) {
            uint32_t imgIndex;
            m_swapchains[eye].acquireImage(&imgIndex);
            m_swapchains[eye].waitImage();

            GLuint framebuffer = m_swapchains[eye].getFramebuffer(imgIndex);
            uint32_t width = m_swapchains[eye].getWidth();
            uint32_t height = m_swapchains[eye].getHeight();

            if (connectionLost) {
                // Show dark overlay with reconnect indication.
                // Timewarp continues on last frame to prevent VR sickness.
                // Tint the rendered output darker to signal connection issue.
                m_renderer.renderSolidColor(framebuffer, width, height,
                    0.02f, 0.02f, 0.04f); // Near-black: "connection lost"
                m_swapchains[eye].releaseImage();

                projViews[eye] = {XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW};
                projViews[eye].pose = views[eye].pose;
                projViews[eye].fov = views[eye].fov;
                projViews[eye].subImage.swapchain = m_swapchains[eye].getHandle();
                projViews[eye].subImage.imageRect.offset = {0, 0};
                projViews[eye].subImage.imageRect.extent = {
                    (int32_t)width, (int32_t)height};
                projViews[eye].subImage.imageArrayIndex = 0;
                continue;
            }

            if (m_hasDecodedFrame && m_lastDecodedTexture != 0) {
                // Rotated by how far the head turned since the frame was
                // rendered (v6) — or, not knowing that, since it was first
                // shown, so a new frame is shown as it came.
                // REGRESSION: the time from rendering to showing a frame
                // (encode, network, decode) was never corrected for.
                const XrFovf& fov = views[eye].fov;
                const fvp_video::Tangents tangents = fvp_video::tangents(
                    fov.angleLeft, fov.angleRight, fov.angleDown, fov.angleUp);
                float rotation[9];
                if (hasNewFrame && !m_frameHasRenderPose) {
                    std::copy(std::begin(fvp_video::kIdentity), std::end(fvp_video::kIdentity), rotation);
                } else {
                    const XrQuaternionf& now = views[eye].pose.orientation;
                    fvp_video::reprojectionRotation(
                        {m_frameOrientation.x, m_frameOrientation.y, m_frameOrientation.z, m_frameOrientation.w},
                        {now.x, now.y, now.z, now.w}, rotation);
                }
                m_renderer.renderVideoFrame(framebuffer, width, height, m_lastDecodedTexture,
                    fvp_video::eyeRect(m_videoLayout, static_cast<int>(eye)), rotation, tangents);
            } else {
                // No frame yet: solid color
                m_renderer.renderSolidColor(framebuffer, width, height,
                    0.05f, 0.05f, 0.2f);
            }

            // The PC paused the stream for user inactivity (SLEEP_ENTER).
            if (m_sleeping) {
                m_overlay.renderSleepDimming(framebuffer, width, height, 0.7f);
            }

            // Connection quality overlay (signal bars)
            if (streaming) {
                // Counters since the last heartbeat (500 ms).
                StatsReporter& stats = m_stream.stats();
                float loss = (float)stats.packetsLost() /
                    std::max(1u, stats.packetsReceived() + stats.packetsLost());
                float quality = 1.0f - std::min(1.0f, loss * 10.0f); // 10% loss = 0 quality
                m_overlay.render(framebuffer, width, height,
                                  quality, loss * 100.0f, m_videoDecoder.avgDecodeLatencyUs() / 1000.0f);

                // HMD dashboard overlay (toggled via menu button)
                m_overlay.renderDashboard(framebuffer, width, height,
                    m_dashboardBitrate, m_dashboardCodecH265, -1);
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
    // Runs up to three times (APP_CMD_DESTROY, end of android_main, the
    // destructor), so every handle is reset once released.
    m_running = false;

    // Leave the PC with DISCONNECT (the engine then ends the session instead
    // of holding it for a reconnect), then stop the receive threads.
    m_stream.stop();
    m_videoReceiver.stop();
    m_trackingSender.shutdown();
    m_audioReceiver.shutdown();
    if (m_audioPlayer.isInitialized()) m_audioPlayer.shutdown();
    // Before EGL goes away: the decoder owns a GL texture.
    if (m_eglDisplay != EGL_NO_DISPLAY) m_videoDecoder.shutdown();
    // Their spaces and action sets belong to the session and instance.
    m_controllerPoller.shutdown();
    m_eyeTracker.shutdown();
    m_actionSets.clear();
    if (m_stageSpace != XR_NULL_HANDLE) {
        xrDestroySpace(m_stageSpace);
        m_stageSpace = XR_NULL_HANDLE;
    }
    if (m_viewSpace != XR_NULL_HANDLE) {
        xrDestroySpace(m_viewSpace);
        m_viewSpace = XR_NULL_HANDLE;
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
        m_eglSurface = EGL_NO_SURFACE;
        m_eglContext = EGL_NO_CONTEXT;
        m_eglDisplay = EGL_NO_DISPLAY;
    }
    LOGI("OpenXR app shut down");
}
