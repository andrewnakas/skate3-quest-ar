@echo off
cd /d "%~dp0"
if "%~1"=="" (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Launch.ps1"
) else (
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Launch.ps1" -Map "%~1"
)
if errorlevel 1 pause
