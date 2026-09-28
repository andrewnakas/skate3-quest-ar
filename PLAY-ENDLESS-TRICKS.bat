@echo off
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Launch-EndlessTricks.ps1" %*
if errorlevel 1 pause
