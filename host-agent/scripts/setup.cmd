@echo off
cd /d "%~dp0"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0windows-host-setup.ps1" %*
if errorlevel 1 (
  echo.
  echo Setup did not finish. Read the message above.
  pause
)
