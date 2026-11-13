@echo off
setlocal
set "SCRIPT_DIR=%~dp0"
if exist "%SCRIPT_DIR%ccide.ps1" (
    set "TARGET_PS1=%SCRIPT_DIR%ccide.ps1"
) else (
    set "TARGET_PS1=%USERPROFILE%\.claude\scripts\ccide.ps1"
)
where pwsh >nul 2>nul
if %ERRORLEVEL% EQU 0 (
    pwsh -NoProfile -ExecutionPolicy Bypass -File "%TARGET_PS1%" %*
    exit /b %ERRORLEVEL%
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%TARGET_PS1%" %*
exit /b %ERRORLEVEL%
