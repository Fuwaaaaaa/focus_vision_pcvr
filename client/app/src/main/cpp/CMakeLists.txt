cmake_minimum_required(VERSION 3.25)
project(focus_vision_client LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Android NDK libraries
find_library(LOG_LIB log)
find_library(ANDROID_LIB android)
find_library(GLES_LIB GLESv3)
find_library(EGL_LIB EGL)
find_library(MEDIANDK_LIB mediandk)
find_library(AAUDIO_LIB aaudio)

# libopus for audio decoding
# Built from source via FetchContent (no prebuilt dependency)
include(FetchContent)
FetchContent_Declare(
    opus
    GIT_REPOSITORY https://gitlab.xiph.org/xiph/opus.git
    GIT_TAG        v1.5.2
    GIT_SHALLOW    TRUE
)
set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(opus)

# OpenXR headers (from third_party or system)
set(OPENXR_INCLUDE_DIR "${CMAKE_SOURCE_DIR}/../../../../../../third_party/openxr/include")

# Source files
set(CLIENT_SOURCES
    main.cpp
    openxr_app.cpp
    xr_swapchain.cpp
    renderer.cpp
    network_receiver.cpp
    video_decoder.cpp
    tcp_client.cpp
    tracking_sender.cpp
    controller_poller.cpp
    timewarp.cpp
    fec_decoder.cpp
    audio_player.cpp
    ${ANDROID_NDK}/sources/android/native_app_glue/android_native_app_glue.c
)

add_library(focusvision_native SHARED ${CLIENT_SOURCES})

target_include_directories(focusvision_native PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${ANDROID_NDK}/sources/android/native_app_glue
    ${OPENXR_INCLUDE_DIR}
)

target_link_libraries(focusvision_native
    ${LOG_LIB}
    ${ANDROID_LIB}
    ${GLES_LIB}
    ${EGL_LIB}
    ${MEDIANDK_LIB}
    ${AAUDIO_LIB}
    opus
    android
)
