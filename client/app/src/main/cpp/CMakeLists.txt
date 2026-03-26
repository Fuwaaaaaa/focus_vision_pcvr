cmake_minimum_required(VERSION 3.25)
project(focus_vision_client LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
find_library(LOG_LIB log)
find_library(ANDROID_LIB android)
find_library(GLES_LIB GLESv3)
find_library(EGL_LIB EGL)
find_library(MEDIANDK_LIB mediandk)

# Stub until Step 4 adds real sources
file(WRITE "${CMAKE_CURRENT_SOURCE_DIR}/stub.cpp"
    "#include <android_native_app_glue.h>\nvoid android_main(struct android_app* app) {}\n")
add_library(focusvision_native SHARED stub.cpp
    ${ANDROID_NDK}/sources/android/native_app_glue/android_native_app_glue.c)
target_include_directories(focusvision_native PRIVATE
    ${ANDROID_NDK}/sources/android/native_app_glue)
target_link_libraries(focusvision_native ${LOG_LIB} ${ANDROID_LIB} android)
