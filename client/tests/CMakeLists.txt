# Host-buildable test harness for the Android client's hardware-independent
# logic. Built with a HOST toolchain (not the Android NDK), mirroring the
# driver's gtest setup (driver/CMakeLists.txt). Only dependency-free sources /
# headers from app/src/main/cpp may be included here.
#
#   cmake -S client/tests -B client/tests/build
#   cmake --build client/tests/build --config Release
#   ctest --test-dir client/tests/build -C Release --output-on-failure
cmake_minimum_required(VERSION 3.18)
project(focus_vision_client_tests LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# --- GoogleTest (same version as the driver tests) ---
include(FetchContent)
FetchContent_Declare(
    googletest
    GIT_REPOSITORY https://github.com/google/googletest.git
    GIT_TAG v1.15.2
)
set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

enable_testing()

add_executable(client_tests
    test_client_protocol.cpp
    test_client_session.cpp
)
# Pure, host-buildable client logic lives alongside the NDK sources.
target_include_directories(client_tests PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../app/src/main/cpp)
target_link_libraries(client_tests PRIVATE GTest::gtest_main)
if(MSVC)
    target_compile_options(client_tests PRIVATE /utf-8)
endif()

include(GoogleTest)
gtest_discover_tests(client_tests)
