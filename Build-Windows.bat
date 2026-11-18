@echo off
setlocal

powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Build-Windows.ps1" %*
set "buildExit=%ERRORLEVEL%"

echo(
if not "%buildExit%"=="0" echo Build failed with exit code %buildExit%.
pause
exit /b %buildExit%
