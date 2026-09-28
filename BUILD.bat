@echo off
setlocal
cd /d "%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Build.ps1"
if errorlevel 1 pause
