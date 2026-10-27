@echo off
rem Makes Skate3-Quest.zip from your own converted Skate 3 disc (see make-bundle.ps1).
rem Put this file and make-bundle.ps1 in the folder with skate3rust.exe and double-click it.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0make-bundle.ps1" %*
echo.
pause
