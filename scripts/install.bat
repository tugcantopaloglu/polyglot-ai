@echo off
REM Polyglot-AI Local - One-Line Installer for Windows
REM
REM Usage: Just double-click this file or run from command prompt
REM

echo.
echo ============================================
echo   Polyglot-AI Local Installer
echo ============================================
echo.

REM Check for PowerShell
where powershell >nul 2>nul
if %errorlevel% neq 0 (
    echo [ERROR] PowerShell is required but not found.
    echo Please install PowerShell and try again.
    pause
    exit /b 1
)

REM Ask about AI tools
echo Do you want to also install AI CLI tools?
echo (Claude Code, Gemini CLI, Codex CLI, GitHub Copilot)
echo.
set /p INSTALL_TOOLS="Install AI tools? [y/N]: "

if /i "%INSTALL_TOOLS%"=="y" (
    set POLYGLOT_WITH_TOOLS=1
) else if /i "%INSTALL_TOOLS%"=="yes" (
    set POLYGLOT_WITH_TOOLS=1
)

echo.
echo Starting installation...
echo.

REM Run PowerShell installer
powershell -ExecutionPolicy Bypass -File "%~dp0install.ps1"

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] Installation failed with error code %errorlevel%
    pause
    exit /b %errorlevel%
)

echo.
pause
