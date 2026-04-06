#pragma once

// Platform defines required before including openxr/openxr_platform.h.
// Include this header before any OpenXR headers in files that use
// platform-specific XR types (e.g., XrSwapchainImageOpenGLESKHR).

#include <jni.h>
#include <EGL/egl.h>
#include <GLES3/gl3.h>

#ifndef XR_USE_GRAPHICS_API_OPENGL_ES
#define XR_USE_GRAPHICS_API_OPENGL_ES
#endif

#ifndef XR_USE_PLATFORM_ANDROID
#define XR_USE_PLATFORM_ANDROID
#endif
