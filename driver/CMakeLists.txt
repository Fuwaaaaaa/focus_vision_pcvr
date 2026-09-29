cmake_minimum_required(VERSION 3.25)
project(focus_vision_pcvr_driver LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(OPENVR_INCLUDE_DIR "${CMAKE_SOURCE_DIR}/../third_party/openvr/headers")
set(RUST_TARGET_DIR "${CMAKE_SOURCE_DIR}/../target/release")
set(RUST_INCLUDE_DIR "${CMAKE_SOURCE_DIR}/../rust/streaming-engine/include")
# Official NVENC API header (nv-codec-headers n12.2.72.0, MIT).
set(NVENC_INCLUDE_DIR "${CMAKE_SOURCE_DIR}/../third_party/nvenc")

file(GLOB DRIVER_SOURCES "src/*.cpp")

if(DRIVER_SOURCES)
    add_library(driver_focus_vision_pcvr SHARED ${DRIVER_SOURCES})
    target_include_directories(driver_focus_vision_pcvr PRIVATE
        ${OPENVR_INCLUDE_DIR} ${RUST_INCLUDE_DIR} ${NVENC_INCLUDE_DIR} src)
    if(MSVC)
        target_compile_options(driver_focus_vision_pcvr PRIVATE /utf-8)
    endif()
    # runtimeobject: cpal's WASAPI backend pulls in windows-core, whose WinRT
    # factory cache calls RoGetActivationFactory. The Rust side expects that
    # import to come from the windows-targets import lib (windows.0.52.0.lib),
    # which this CMake build does not link — without runtimeobject the DLL
    # fails with LNK2019 __imp_RoGetActivationFactory.
    target_link_libraries(driver_focus_vision_pcvr PRIVATE
        "${RUST_TARGET_DIR}/streaming_engine.lib"
        ws2_32 userenv ntdll bcrypt d3d11 dxgi d3dcompiler propsys ole32 oleaut32 runtimeobject)
    # SteamVR loads the DLL from <driver>/bin/win64/. A multi-config
    # generator (Visual Studio, used by CI and build.bat) appends a
    # per-config folder (bin/win64/Release/) unless the path contains a
    # generator expression, so wrap it in one.
    set_target_properties(driver_focus_vision_pcvr PROPERTIES
        OUTPUT_NAME "driver_focus_vision_pcvr"
        RUNTIME_OUTPUT_DIRECTORY "$<1:${CMAKE_BINARY_DIR}/focus_vision_pcvr/bin/win64>")
    add_custom_command(TARGET driver_focus_vision_pcvr POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy
            "${CMAKE_SOURCE_DIR}/driver.vrdrivermanifest"
            "${CMAKE_BINARY_DIR}/focus_vision_pcvr/"
        COMMAND ${CMAKE_COMMAND} -E copy_directory
            "${CMAKE_SOURCE_DIR}/resources"
            "${CMAKE_BINARY_DIR}/focus_vision_pcvr/resources")
endif()

# --- GoogleTest for hardware-independent logic ---
include(FetchContent)
FetchContent_Declare(
    googletest
    GIT_REPOSITORY https://github.com/google/googletest.git
    GIT_TAG v1.15.2
)
set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(googletest)

enable_testing()

add_executable(driver_tests
    tests/test_qp_map.cpp
    tests/test_nvenc_vui.cpp
    tests/test_nvenc_config.cpp
    tests/test_encode_params.cpp
    tests/test_display_geometry.cpp
    # The D3D11 side of direct mode, run on WARP (no GPU needed).
    tests/test_d3d_pipeline.cpp
    src/swap_textures.cpp
    src/eye_blit.cpp
    src/sync_texture.cpp
)
target_include_directories(driver_tests PRIVATE src ${NVENC_INCLUDE_DIR})
target_link_libraries(driver_tests PRIVATE GTest::gtest_main d3d11 dxgi d3dcompiler)
if(MSVC)
    target_compile_options(driver_tests PRIVATE /utf-8)
endif()

include(GoogleTest)
gtest_discover_tests(driver_tests)
