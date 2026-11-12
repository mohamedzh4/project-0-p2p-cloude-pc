@echo off
REM Reverse tunnel: Azure Matchmaker dials 127.0.0.1:17842 on the VPS.
cd /d "%~dp0"
if not exist "%~dp0tunnel.pem" (
  echo Missing tunnel.pem
  exit /b 1
)
ssh.exe -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -o StrictHostKeyChecking=accept-new -o IdentitiesOnly=yes -i "%~dp0tunnel.pem" -R 127.0.0.1:17842:127.0.0.1:7842 dcl-tunnel@203.0.113.10
