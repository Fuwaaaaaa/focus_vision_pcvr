@echo off
setlocal

set "DRIVER_DIR=%~dp0build\focus_vision_pcvr"
set "VRPATHREG=%ProgramFiles(x86)%\Steam\steamapps\common\SteamVR\bin\win64\vrpathreg.exe"

if not exist "%VRPATHREG%" (
    echo ERROR: vrpathreg.exe not found. Is SteamVR installed?
    echo Expected path: %VRPATHREG%
    pause
    exit /b 1
)

if not exist "%DRIVER_DIR%\driver.vrdrivermanifest" (
    echo ERROR: Driver not built. Run build.bat first.
    pause
    exit /b 1
)

echo Registering Focus Vision PCVR driver with SteamVR...
"%VRPATHREG%" adddriver "%DRIVER_DIR%"

if %ERRORLEVEL% == 0 (
    echo Driver registered successfully.
    echo Restart SteamVR to load the driver.
) else (
    echo ERROR: Failed to register driver.
)

pause
