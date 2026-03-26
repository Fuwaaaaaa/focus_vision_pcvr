@echo off
setlocal enabledelayedexpansion

echo ============================================
echo  Focus Vision PCVR - Build Script
echo ============================================

where cargo >nul 2>&1 || (echo ERROR: Rust/Cargo not found. & exit /b 1)

echo.
echo [1/2] Building Rust streaming engine...
cargo build --release -p streaming-engine
if %ERRORLEVEL% neq 0 (echo ERROR: Rust build failed. & exit /b 1)
echo Rust build OK.

echo.
echo [2/2] Building OpenVR driver...
where cmake >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: CMake not found. Install CMake to build the driver DLL.
) else (
    if exist driver\src\driver_main.cpp (
        cmake -B driver\build -S driver -DCMAKE_BUILD_TYPE=Release
        cmake --build driver\build --config Release
        if %ERRORLEVEL% neq 0 (echo ERROR: CMake build failed. & exit /b 1)
        echo Driver build OK.
    ) else (
        echo SKIP: No driver sources yet.
    )
)

if not exist out mkdir out
if exist target\release\streaming_engine.lib copy /Y target\release\streaming_engine.lib out\ >nul

echo.
echo Build complete! Artifacts in: out\
