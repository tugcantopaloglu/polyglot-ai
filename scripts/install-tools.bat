@echo off
REM Polyglot-AI Tools Installation Script for Windows
REM This batch file runs the PowerShell installation script

echo.
echo ============================================
echo Polyglot-AI Tools Installer for Windows
echo ============================================
echo.

REM Check if running as administrator
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo [WARN] Not running as administrator.
    echo [WARN] Some installations may require elevated privileges.
    echo.
)

REM Run PowerShell script
powershell -ExecutionPolicy Bypass -File "%~dp0install-tools.ps1" %*

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] Installation script failed with error code %errorlevel%
    pause
    exit /b %errorlevel%
)

echo.
pause
