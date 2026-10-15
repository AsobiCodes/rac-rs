@echo off
rem Double-click to set up the game from your own disc (see README.md). A small window opens; no console window stays open.
rem For troubleshooting, run "Setup.cmd -Console" to see the old console output.
if /i "%~1"=="-Console" goto console
start "" powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0setup.ps1" %*
exit /b
:console
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1" %*
echo.
pause
