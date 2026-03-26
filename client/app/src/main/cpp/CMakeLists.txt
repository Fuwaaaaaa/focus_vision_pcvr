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

# Source files (added in Step 4)
set(CLIENT_SOURCES "")

# Placeholder: will be populated in Step 4
# For now, create a minimal native library stub
file(GLOB CLIENT_SOURCES "*.cpp")

if(CLIENT_SOURCES)
    add_library(focusvision_native SHARED ${CLIENT_SOURCES})

    target_link_libraries(focusvision_native
        ${LOG_LIB}
        ${ANDROID_LIB}
        ${GLES_LIB}
        ${EGL_LIB}
        ${MEDIANDK_LIB}
        android
    )
else()
    # Minimal stub so Gradle doesn't fail
    file(WRITE "${CMAKE_CURRENT_SOURCE_DIR}/stub.cpp"
        "#include <jni.h>\nextern \"C\" void android_main(struct android_app* app) {}\n")
    add_library(focusvision_native SHARED stub.cpp)
    target_link_libraries(focusvision_native ${LOG_LIB} ${ANDROID_LIB} android)
endif()
