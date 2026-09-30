# Host-buildable test harness for the Android client's hardware-independent
# logic. Built with a HOST toolchain (not the Android NDK), mirroring the
# driver's gtest setup (driver/CMakeLists.txt). Only sources free of OpenXR,
# EGL and MediaCodec from app/src/main/cpp may be included here.
#
#   cmake -S client/tests -B client/tests/build
#   cmake --build client/tests/build --config Release
#   ctest --test-dir client/tests/build -C Release --output-on-failure
#
# test_session_e2e.cpp runs the client's networking (StreamSession +
# VideoReceiver) against the real engine and is skipped unless
# FVP_HEADLESS_BIN names a focus-vision-headless binary:
#   cargo build --release -p streaming-engine --features simulator --bins
#   FVP_HEADLESS_BIN=target/release/focus-vision-headless(.exe) ctest ...
cmake_minimum_required(VERSION 3.18)
project(focus_vision_client_tests LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

# --- GoogleTest (same version as the driver tests) ---
FetchContent_Declare(
    googletest
    GIT_REPOSITORY https://github.com/google/googletest.git
    GIT_TAG v1.15.2
)
set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

# --- MbedTLS (same version as the app, app/src/main/cpp/CMakeLists.txt) ---
FetchContent_Declare(
    mbedtls
    GIT_REPOSITORY https://github.com/Mbed-TLS/mbedtls.git
    GIT_TAG        v3.6.2
    GIT_SHALLOW    TRUE
)
set(ENABLE_TESTING OFF CACHE BOOL "" FORCE)
set(ENABLE_PROGRAMS OFF CACHE BOOL "" FORCE)
set(MBEDTLS_FATAL_WARNINGS OFF CACHE BOOL "" FORCE)
set(MSVC_STATIC_RUNTIME OFF CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(mbedtls)

find_package(Threads REQUIRED)

enable_testing()

set(CLIENT_SRC ${CMAKE_CURRENT_SOURCE_DIR}/../app/src/main/cpp)

add_executable(client_tests
    test_audio_sequence.cpp
    test_client_protocol.cpp
    test_client_session.cpp
    test_fec_decoder.cpp
    test_frame_assembler.cpp
    test_launch_request.cpp
    test_video_view.cpp
    test_openxr_extensions.cpp
    test_render_pose_log.cpp
    test_session_e2e.cpp
    # NDK sources compiled for the host. Their only platform dependencies
    # (logging, XrResult via xr_utils.h) are stubbed in shim/ below; sockets
    # go through net_compat.h (POSIX or Winsock).
    ${CLIENT_SRC}/fec_decoder.cpp
    ${CLIENT_SRC}/frame_assembler.cpp
    ${CLIENT_SRC}/video_receiver.cpp
    ${CLIENT_SRC}/tcp_client.cpp
    ${CLIENT_SRC}/stream_session.cpp
)
# Pure, host-buildable client logic lives alongside the NDK sources. shim/
# provides host stand-ins for <android/log.h> and <openxr/openxr.h>.
target_include_directories(client_tests PRIVATE
    ${CLIENT_SRC}
    ${CMAKE_CURRENT_SOURCE_DIR}/shim)
target_link_libraries(client_tests PRIVATE
    GTest::gtest_main mbedtls mbedx509 mbedcrypto Threads::Threads)
if(WIN32)
    target_link_libraries(client_tests PRIVATE ws2_32)
endif()
if(MSVC)
    target_compile_options(client_tests PRIVATE /utf-8)
endif()

include(GoogleTest)
# The E2E tests start an engine and stream for a few seconds each.
gtest_discover_tests(client_tests DISCOVERY_TIMEOUT 60 PROPERTIES TIMEOUT 120)
