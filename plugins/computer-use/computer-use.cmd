@echo off
set "DIR=%~dp0"
if exist "%ProgramFiles%\nodejs\node.exe" (
  "%ProgramFiles%\nodejs\node.exe" "%DIR%server.mjs"
  exit /b %ERRORLEVEL%
)
where node >nul 2>&1
if errorlevel 1 (
  echo NODE_MISSING 1>&2
  exit /b 1
)
node "%DIR%server.mjs"
