@echo off
setlocal
set "DRIVER_DIR=%~dp0build\focus_vision_pcvr"
set "VRPATHREG=%ProgramFiles(x86)%\Steam\steamapps\common\SteamVR\bin\win64\vrpathreg.exe"
if not exist "%VRPATHREG%" (echo ERROR: vrpathreg not found. & pause & exit /b 1)
if not exist "%DRIVER_DIR%\driver.vrdrivermanifest" (echo ERROR: Build first. & pause & exit /b 1)
"%VRPATHREG%" adddriver "%DRIVER_DIR%"
if %ERRORLEVEL% == 0 (echo Driver registered. Restart SteamVR.) else (echo ERROR: Registration failed.)
pause
