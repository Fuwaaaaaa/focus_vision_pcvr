#include <android_native_app_glue.h>
#include "openxr_app.h"
#include "xr_utils.h"

static OpenXRApp* g_app = nullptr;

static void handleAppCmd(android_app* app, int32_t cmd) {
    switch (cmd) {
    case APP_CMD_INIT_WINDOW:
        LOGI("APP_CMD_INIT_WINDOW");
        break;
    case APP_CMD_TERM_WINDOW:
        LOGI("APP_CMD_TERM_WINDOW");
        break;
    case APP_CMD_DESTROY:
        LOGI("APP_CMD_DESTROY");
        if (g_app) g_app->shutdown();
        break;
    default:
        break;
    }
}

void android_main(struct android_app* app) {
    LOGI("=== Focus Vision PCVR Client Starting ===");

    app->onAppCmd = handleAppCmd;

    OpenXRApp xrApp;
    g_app = &xrApp;

    try {
        xrApp.initialize(app);

        while (!app->destroyRequested && xrApp.isRunning()) {
            // Process Android events
            int events;
            struct android_poll_source* source;
            while (ALooper_pollAll(0, nullptr, &events, (void**)&source) >= 0) {
                if (source) source->process(app, source);
            }

            // Run OpenXR frame loop
            if (xrApp.isRunning()) {
                xrApp.mainLoop();
            }
        }
    } catch (const std::exception& e) {
        LOGE("Fatal error: %s", e.what());
    }

    xrApp.shutdown();
    g_app = nullptr;
    LOGI("=== Focus Vision PCVR Client Exiting ===");
}
