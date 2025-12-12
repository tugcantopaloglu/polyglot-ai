#Requires -Version 5.1
<#
.SYNOPSIS
    Polyglot-AI Local - One-Line Installer for Windows

.DESCRIPTION
    Automatically installs Polyglot-AI Local with all dependencies.

.PARAMETER WithTools
    Also install AI CLI tools (Claude, Gemini, Codex, Copilot)

.PARAMETER SkipRust
    Skip Rust installation (if already installed)

.PARAMETER InstallDir
    Installation directory (default: $env:USERPROFILE\.polyglot-ai)

.PARAMETER Version
    Version to install (default: latest)

.EXAMPLE
    # Basic installation
    irm https://raw.githubusercontent.com/tugcantopaloglu/selfhosted-ai-code-platform/main/scripts/install.ps1 | iex

    # With AI tools
    $env:POLYGLOT_WITH_TOOLS = "1"; irm ... | iex

.NOTES
    Run in PowerShell as Administrator for best results.
#>

param(
    [switch]$WithTools,
    [switch]$SkipRust,
    [string]$InstallDir = "$env:USERPROFILE\.polyglot-ai",
    [string]$Version = "latest"
)

# Check for environment variable overrides
if ($env:POLYGLOT_WITH_TOOLS -eq "1") { $WithTools = $true }
if ($env:POLYGLOT_INSTALL_DIR) { $InstallDir = $env:POLYGLOT_INSTALL_DIR }
if ($env:POLYGLOT_VERSION) { $Version = $env:POLYGLOT_VERSION }

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$RepoUrl = "https://github.com/tugcantopaloglu/selfhosted-ai-code-platform"
$BinDir = "$InstallDir\bin"

function Write-Banner {
    Write-Host ""
    Write-Host "  ____       _             _       _        _    ___ " -ForegroundColor Cyan
    Write-Host " |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|" -ForegroundColor Cyan
    Write-Host " | |_) / _ \| | | | |/ _`` | |/ _ \| __|   / _ \  | | " -ForegroundColor Cyan
    Write-Host " |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | " -ForegroundColor Cyan
    Write-Host " |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|" -ForegroundColor Cyan
    Write-Host "               |___/ |___/                           " -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Polyglot-AI Local Installer for Windows" -ForegroundColor Green
    Write-Host ""
}

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] " -ForegroundColor Blue -NoNewline
    Write-Host $Message
}

function Write-Success {
    param([string]$Message)
    Write-Host "[OK] " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] " -ForegroundColor Red -NoNewline
    Write-Host $Message
}

function Test-Command {
    param([string]$Command)
    $null = Get-Command $Command -ErrorAction SilentlyContinue
    return $?
}

function Refresh-Path {
    $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
                [System.Environment]::GetEnvironmentVariable("Path", "User")
}

function Install-Rust {
    if (Test-Command "cargo") {
        $rustVersion = rustc --version
        Write-Success "Rust already installed: $rustVersion"
        return
    }

    if ($SkipRust) {
        throw "Rust is required but -SkipRust was specified"
    }

    Write-Info "Installing Rust..."

    # Download and run rustup-init
    $rustupInit = "$env:TEMP\rustup-init.exe"
    Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit

    # Run installer silently
    Start-Process -FilePath $rustupInit -ArgumentList "-y", "--default-toolchain", "stable" -Wait -NoNewWindow

    # Refresh PATH
    $env:Path += ";$env:USERPROFILE\.cargo\bin"
    Refresh-Path

    if (Test-Command "cargo") {
        Write-Success "Rust installed successfully"
    } else {
        throw "Failed to install Rust. Please install manually from https://rustup.rs"
    }
}

function Install-Git {
    if (Test-Command "git") {
        Write-Success "Git already installed"
        return
    }

    Write-Info "Installing Git..."

    if (Test-Command "winget") {
        winget install -e --id Git.Git --accept-source-agreements --accept-package-agreements
        Refresh-Path
    } else {
        throw "Git is required. Please install Git from https://git-scm.com/download/win"
    }

    if (Test-Command "git") {
        Write-Success "Git installed successfully"
    } else {
        throw "Failed to install Git"
    }
}

function Install-NodeJS {
    if (Test-Command "node") {
        $nodeVersion = node --version
        Write-Success "Node.js already installed: $nodeVersion"
        return
    }

    Write-Info "Installing Node.js..."

    if (Test-Command "winget") {
        winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
        Refresh-Path
    } else {
        # Download and install directly
        $nodeInstaller = "$env:TEMP\node-installer.msi"
        Invoke-WebRequest -Uri "https://nodejs.org/dist/v20.10.0/node-v20.10.0-x64.msi" -OutFile $nodeInstaller
        Start-Process -FilePath "msiexec.exe" -ArgumentList "/i", $nodeInstaller, "/quiet", "/norestart" -Wait
        Refresh-Path
    }

    if (Test-Command "node") {
        Write-Success "Node.js installed successfully"
    } else {
        Write-Warning "Node.js installation may have failed. AI tools may not work."
    }
}

function Install-VisualStudioBuildTools {
    # Check if build tools are available
    $vsWhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vsWhere) {
        $vsPath = & $vsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vsPath) {
            Write-Success "Visual Studio Build Tools found"
            return
        }
    }

    # Check for standalone build tools
    if (Test-Path "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools") {
        Write-Success "Visual Studio Build Tools found"
        return
    }

    Write-Info "Installing Visual Studio Build Tools (required for Rust)..."
    Write-Warning "This may take several minutes..."

    if (Test-Command "winget") {
        winget install -e --id Microsoft.VisualStudio.2022.BuildTools --accept-source-agreements --accept-package-agreements --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    } else {
        Write-Warning "Please install Visual Studio Build Tools manually from:"
        Write-Host "https://visualstudio.microsoft.com/visual-cpp-build-tools/"
        Write-Host ""
        Write-Host "Select 'Desktop development with C++' workload"
        throw "Visual Studio Build Tools required"
    }

    Write-Success "Visual Studio Build Tools installed"
}

function Clone-AndBuild {
    Write-Info "Creating installation directory..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null

    $repoPath = "$InstallDir\polyglot-ai"

    if (Test-Path $repoPath) {
        Write-Info "Updating existing installation..."
        Push-Location $repoPath
        git pull
    } else {
        Write-Info "Cloning repository..."
        Push-Location $InstallDir
        git clone $RepoUrl polyglot-ai
        Set-Location polyglot-ai
    }

    if ($Version -ne "latest") {
        Write-Info "Checking out version: $Version"
        git checkout $Version
    }

    Write-Info "Building polyglot-local (this may take several minutes)..."
    cargo build --release -p polyglot-local

    # Copy binary
    Write-Info "Installing binary..."
    Copy-Item "target\release\polyglot-local.exe" "$BinDir\" -Force

    Pop-Location

    Write-Success "polyglot-local installed to $BinDir\polyglot-local.exe"
}

function Install-AITools {
    if (-not $WithTools) {
        return
    }

    Write-Info "Installing AI CLI tools..."

    if (-not (Test-Command "npm")) {
        Write-Warning "npm not found. Skipping AI tool installation."
        return
    }

    # Install Claude Code
    Write-Info "Installing Claude Code CLI..."
    try {
        npm install -g @anthropic-ai/claude-code 2>$null
        Write-Success "Claude Code installed"
    } catch {
        Write-Warning "Claude Code installation failed"
    }

    # Install Gemini CLI
    Write-Info "Installing Gemini CLI..."
    try {
        npm install -g @google/gemini-cli 2>$null
        Write-Success "Gemini CLI installed"
    } catch {
        Write-Warning "Gemini CLI installation failed"
    }

    # Install Codex CLI
    Write-Info "Installing Codex CLI..."
    try {
        npm install -g @openai/codex-cli 2>$null
        Write-Success "Codex CLI installed"
    } catch {
        Write-Warning "Codex CLI installation failed"
    }

    # Install GitHub Copilot CLI
    if (Test-Command "gh") {
        Write-Info "Installing GitHub Copilot extension..."
        try {
            gh extension install github/gh-copilot 2>$null
            Write-Success "GitHub Copilot installed"
        } catch {
            Write-Warning "Copilot installation failed"
        }
    } else {
        Write-Warning "GitHub CLI (gh) not found. Skipping Copilot installation."
    }
}

function Setup-Path {
    Write-Info "Setting up PATH..."

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")

    if ($userPath -notlike "*$BinDir*") {
        [Environment]::SetEnvironmentVariable(
            "Path",
            "$userPath;$BinDir",
            "User"
        )
        $env:Path += ";$BinDir"
        Write-Success "Added $BinDir to PATH"
    } else {
        Write-Success "PATH already configured"
    }
}

function Verify-Installation {
    Write-Info "Verifying installation..."

    $binary = "$BinDir\polyglot-local.exe"
    if (Test-Path $binary) {
        Write-Success "polyglot-local binary is installed"

        Write-Host ""
        Write-Info "Running system check..."
        & $binary doctor
    } else {
        throw "Installation verification failed"
    }
}

function Print-Success {
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Green
    Write-Host "   Installation Complete!" -ForegroundColor Green
    Write-Host "============================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "To get started:" -ForegroundColor White
    Write-Host ""
    Write-Host "  # Open a new terminal, then run:" -ForegroundColor Cyan
    Write-Host "  polyglot-local" -ForegroundColor White
    Write-Host ""
    Write-Host "  # Or send a single prompt:" -ForegroundColor Cyan
    Write-Host '  polyglot-local ask "Write hello world in Python"' -ForegroundColor White
    Write-Host ""
    Write-Host "  # Check available tools:" -ForegroundColor Cyan
    Write-Host "  polyglot-local doctor" -ForegroundColor White
    Write-Host ""

    if (-not $WithTools) {
        Write-Host "Note: AI tools were not installed." -ForegroundColor Yellow
        Write-Host "To install them, run:" -ForegroundColor White
        Write-Host "  $InstallDir\polyglot-ai\scripts\install-tools.ps1" -ForegroundColor Gray
        Write-Host ""
    }

    Write-Host "For more information, visit:" -ForegroundColor White
    Write-Host "  https://github.com/tugcantopaloglu/selfhosted-ai-code-platform" -ForegroundColor Cyan
    Write-Host ""
}

# Main installation
function Main {
    try {
        Write-Banner

        # Check if running as admin (recommended)
        $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
        if (-not $isAdmin) {
            Write-Warning "Not running as Administrator. Some installations may fail."
            Write-Host ""
        }

        Install-Git
        Install-VisualStudioBuildTools
        Install-Rust

        if ($WithTools) {
            Install-NodeJS
        }

        Clone-AndBuild
        Install-AITools
        Setup-Path
        Verify-Installation
        Print-Success

    } catch {
        Write-Host ""
        Write-Error $_.Exception.Message
        Write-Host ""
        Write-Host "Installation failed. Please check the error above." -ForegroundColor Red
        Write-Host "For help, visit: https://github.com/tugcantopaloglu/selfhosted-ai-code-platform/issues" -ForegroundColor White
        exit 1
    }
}

Main
