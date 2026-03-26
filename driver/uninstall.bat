@echo off
setlocal
set "DRIVER_DIR=%~dp0build\focus_vision_pcvr"
set "VRPATHREG=%ProgramFiles(x86)%\Steam\steamapps\common\SteamVR\bin\win64\vrpathreg.exe"
if not exist "%VRPATHREG%" (echo ERROR: vrpathreg not found. & pause & exit /b 1)
"%VRPATHREG%" removedriver "%DRIVER_DIR%"
if %ERRORLEVEL% == 0 (echo Driver unregistered.) else (echo ERROR: Failed.)
pause
