@echo off
setlocal enabledelayedexpansion

echo ============================================
echo  Focus Vision PCVR - Build Script
echo ============================================

:: Check Rust
where cargo >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo ERROR: Rust/Cargo not found. Install from https://rustup.rs
    exit /b 1
)

:: Check CMake
where cmake >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo ERROR: CMake not found. Install from https://cmake.org
    exit /b 1
)

:: Step 1: Build Rust streaming engine
echo.
echo [1/3] Building Rust streaming engine...
cargo build --release -p streaming-engine
if %ERRORLEVEL% neq 0 (
    echo ERROR: Rust build failed.
    exit /b 1
)
echo Rust build OK.

:: Step 2: Build OpenVR driver DLL
echo.
echo [2/3] Building OpenVR driver...
if exist driver\src\driver_main.cpp (
    cmake -B driver\build -S driver -DCMAKE_BUILD_TYPE=Release
    cmake --build driver\build --config Release
    if %ERRORLEVEL% neq 0 (
        echo ERROR: CMake build failed.
        exit /b 1
    )
    echo Driver build OK.
) else (
    echo SKIP: No driver sources yet (Step 2 pending).
)

:: Step 3: Copy artifacts
echo.
echo [3/3] Copying artifacts...
if not exist out mkdir out
if exist target\release\streaming_engine.lib (
    copy /Y target\release\streaming_engine.lib out\ >nul
)
if exist driver\build\focus_vision_pcvr\bin\win64\driver_focus_vision_pcvr.dll (
    xcopy /E /Y driver\build\focus_vision_pcvr out\driver\ >nul
)

echo.
echo ============================================
echo  Build complete!
echo ============================================
echo Artifacts in: out\
