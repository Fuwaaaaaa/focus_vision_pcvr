cmake_minimum_required(VERSION 3.25)
project(focus_vision_pcvr_driver LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# OpenVR SDK headers
set(OPENVR_INCLUDE_DIR "${CMAKE_SOURCE_DIR}/../third_party/openvr/headers")

# Rust streaming engine static library
set(RUST_TARGET_DIR "${CMAKE_SOURCE_DIR}/../target/release")
set(RUST_INCLUDE_DIR "${CMAKE_SOURCE_DIR}/../rust/streaming-engine/include")

# Driver sources (added in Step 2)
file(GLOB DRIVER_SOURCES "src/*.cpp")

if(DRIVER_SOURCES)
    add_library(driver_focus_vision_pcvr SHARED ${DRIVER_SOURCES})

    target_include_directories(driver_focus_vision_pcvr PRIVATE
        ${OPENVR_INCLUDE_DIR}
        ${RUST_INCLUDE_DIR}
        src
    )

    # Link Rust static library
    target_link_libraries(driver_focus_vision_pcvr PRIVATE
        "${RUST_TARGET_DIR}/streaming_engine.lib"
        ws2_32
        userenv
        ntdll
        bcrypt
        d3d11
        dxgi
    )

    # Output to SteamVR driver directory structure
    set_target_properties(driver_focus_vision_pcvr PROPERTIES
        OUTPUT_NAME "driver_focus_vision_pcvr"
        RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/focus_vision_pcvr/bin/win64"
    )

    # Copy manifest and settings to output
    add_custom_command(TARGET driver_focus_vision_pcvr POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy
            "${CMAKE_SOURCE_DIR}/driver.vrdrivermanifest"
            "${CMAKE_BINARY_DIR}/focus_vision_pcvr/"
        COMMAND ${CMAKE_COMMAND} -E copy_directory
            "${CMAKE_SOURCE_DIR}/resources"
            "${CMAKE_BINARY_DIR}/focus_vision_pcvr/resources"
    )
else()
    message(STATUS "No driver sources found yet (Step 2 pending)")
endif()
