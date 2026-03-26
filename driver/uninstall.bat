@echo off
setlocal

set "DRIVER_DIR=%~dp0build\focus_vision_pcvr"
set "VRPATHREG=%ProgramFiles(x86)%\Steam\steamapps\common\SteamVR\bin\win64\vrpathreg.exe"

if not exist "%VRPATHREG%" (
    echo ERROR: vrpathreg.exe not found.
    pause
    exit /b 1
)

echo Unregistering Focus Vision PCVR driver from SteamVR...
"%VRPATHREG%" removedriver "%DRIVER_DIR%"

if %ERRORLEVEL% == 0 (
    echo Driver unregistered successfully.
) else (
    echo ERROR: Failed to unregister driver.
)

pause
