@echo off
setlocal
set "CORE=%CCIDE_CORE_BIN%"
if "%CORE%"=="" set "CORE=%USERPROFILE%\.claude\bin\ccide-core.exe"
if exist "%CORE%" (
  "%CORE%" %*
  exit /b %ERRORLEVEL%
)
echo {}
exit /b 0
