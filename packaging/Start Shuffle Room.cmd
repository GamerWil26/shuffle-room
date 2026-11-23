@echo off
cd /d "%~dp0"
shuffle-room.exe %*
if errorlevel 1 pause
