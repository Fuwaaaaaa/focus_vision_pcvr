@echo off
setlocal enabledelayedexpansion

echo ============================================
echo  Focus Vision PCVR - Build Script
echo ============================================

where cargo >nul 2>&1 || (echo ERROR: Rust/Cargo not found. & exit /b 1)

REM Initialize git submodules (OpenVR SDK etc.)
where git >nul 2>&1 && (
    git submodule update --init --quiet 2>nul
)

echo.
echo [1/4] Building Rust streaming engine...
cargo build --release -p streaming-engine
if %ERRORLEVEL% neq 0 (echo ERROR: Rust build failed. & exit /b 1)
echo Rust build OK.

echo.
echo [2/4] Building simulator binaries (headless, mock-client, etc.) ...
REM Built behind the `simulator` feature so production driver builds stay
REM lean. Used by JSON-driven scenario tests and for local "no hardware"
REM dogfooding. A failure here is informational only — production lib
REM and driver builds below should not be blocked.
cargo build --release -p streaming-engine --features simulator --bins
if %ERRORLEVEL% neq 0 (
    echo WARN: Simulator binary build failed — production artifacts unaffected.
) else (
    echo Simulator binaries OK.
)

echo.
echo [3/4] Building companion app (with in-process simulation)...
REM `--features simulator` so the shipped exe (NSIS bundles target\release\
REM focus-vision.exe) carries the in-process simulation: end users can run the
REM full pipeline with no VR hardware via the "Start Simulation" button /
REM `--simulate`. Adds only hound + tokio + the streaming-engine staticlib.
cargo build --release -p focus-vision-companion --features simulator
if %ERRORLEVEL% neq 0 (echo ERROR: Companion build failed. & exit /b 1)
echo Companion build OK.

echo.
echo [4/4] Building OpenVR driver...
where cmake >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo SKIP: CMake not found. Install CMake to build the driver DLL.
) else (
    if exist driver\src\driver_main.cpp (
        cmake -B driver\build -S driver -DCMAKE_BUILD_TYPE=Release
        if %ERRORLEVEL% neq 0 (echo ERROR: CMake configure failed. & exit /b 1)
        cmake --build driver\build --config Release
        if %ERRORLEVEL% neq 0 (echo ERROR: CMake build failed. & exit /b 1)
        echo Driver build OK.
    ) else (
        echo SKIP: No driver sources yet.
    )
)

if not exist out mkdir out
if exist target\release\streaming_engine.lib copy /Y target\release\streaming_engine.lib out\ >nul
REM Companion exe (sim-enabled). NSIS reads it from target\release\, this copy
REM is just for convenience so `out\` holds a full artifact set.
if exist target\release\focus-vision.exe copy /Y target\release\focus-vision.exe out\ >nul
REM Copy simulator binaries when available so devs can run scenarios
REM from `out\` without re-pointing PATH at `target\release\`.
if exist target\release\focus-vision-headless.exe copy /Y target\release\focus-vision-headless.exe out\ >nul
if exist target\release\focus-vision-mock-client.exe copy /Y target\release\focus-vision-mock-client.exe out\ >nul

echo.
echo Build complete! Artifacts in: out\
