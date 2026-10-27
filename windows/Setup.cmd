@echo off
rem Double-click to set up the game from your own disc (see README.md). A small window opens; no console window stays open.
rem For troubleshooting, run "Setup.cmd -Console" to see the old console output.
if /i "%~1"=="-Console" goto console
if /i "%~1"=="-AddToSteam" goto console
if /i "%~1"=="-RemoveFromSteam" goto console
rem The RAC-RS Launcher is next to this file: open it (it does the same set-up with a nicer window).
if exist "%~dp0RAC-RS Launcher.exe" goto launcher
rem You chose the launcher earlier but it is gone (deleted, or its download failed): fetch it again from its own GitHub project (hidden, a few seconds), as the window would.
rem (for /f, not findstr: findstr /x does not match the last line of a file with LF-only line ends, which is how setup.ps1 writes it)
set "RACL="
if exist "%~dp0generated\setup_choice.txt" for /f "usebackq tokens=1,* delims==" %%a in ("%~dp0generated\setup_choice.txt") do if /i "%%a"=="launcher" if /i "%%b"=="yes" set "RACL=1"
if defined RACL powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0setup.ps1" -GetLauncher
if exist "%~dp0RAC-RS Launcher.exe" goto launcher
rem Otherwise the setup window opens first. It has the box "Also install the RAC-RS Launcher" (ticked at first; the answer is remembered in generated\setup_choice.txt).
start "" powershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0setup.ps1" %*
exit /b
:launcher
start "" "%~dp0RAC-RS Launcher.exe"
exit /b
:console
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1" %*
echo.
pause
