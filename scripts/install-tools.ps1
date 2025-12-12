# Polyglot-AI Tools Installation Script for Windows
# This script installs the AI CLI tools required by Polyglot-AI server

param(
    [switch]$Claude,
    [switch]$Gemini,
    [switch]$Codex,
    [switch]$Copilot,
    [switch]$All
)

$ErrorActionPreference = "Stop"

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "[OK] $Message" -ForegroundColor Green
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARN] $Message" -ForegroundColor Yellow
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

function Test-Command {
    param([string]$Command)
    $null = Get-Command $Command -ErrorAction SilentlyContinue
    return $?
}

function Install-NodeIfNeeded {
    if (-not (Test-Command "node")) {
        Write-Info "Node.js not found. Installing via winget..."
        winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
    }

    if (Test-Command "node") {
        $nodeVersion = node --version
        Write-Success "Node.js $nodeVersion is available"
        return $true
    } else {
        Write-Error "Failed to install Node.js. Please install manually from https://nodejs.org/"
        return $false
    }
}

function Install-Claude {
    Write-Info "Installing Claude Code CLI..."

    if (Test-Command "claude") {
        Write-Success "Claude Code is already installed"
        claude --version
        return $true
    }

    try {
        npm install -g @anthropic-ai/claude-code
        if (Test-Command "claude") {
            Write-Success "Claude Code installed successfully"
            return $true
        }
    } catch {
        Write-Warning "npm install failed, trying alternative method..."
    }

    Write-Warning "Claude Code CLI may require manual installation"
    Write-Info "Visit: https://docs.anthropic.com/claude-code for installation instructions"
    return $false
}

function Install-Gemini {
    Write-Info "Installing Gemini CLI..."

    if (Test-Command "gemini") {
        Write-Success "Gemini CLI is already installed"
        gemini --version
        return $true
    }

    try {
        npm install -g @google/gemini-cli
        if (Test-Command "gemini") {
            Write-Success "Gemini CLI installed successfully"
            return $true
        }
    } catch {
        Write-Warning "npm install failed"
    }

    Write-Warning "Gemini CLI may require manual installation"
    Write-Info "Visit: https://ai.google.dev/gemini-api/docs/gemini-cli for installation instructions"
    return $false
}

function Install-Codex {
    Write-Info "Installing OpenAI Codex CLI..."

    if (Test-Command "codex") {
        Write-Success "Codex CLI is already installed"
        codex --version
        return $true
    }

    try {
        npm install -g @openai/codex-cli
        if (Test-Command "codex") {
            Write-Success "Codex CLI installed successfully"
            return $true
        }
    } catch {
        Write-Warning "npm install failed"
    }

    Write-Warning "Codex CLI may require manual installation"
    Write-Info "Visit: https://platform.openai.com/docs/codex for installation instructions"
    return $false
}

function Install-Copilot {
    Write-Info "Installing GitHub Copilot CLI..."

    # Check if gh is installed
    if (-not (Test-Command "gh")) {
        Write-Info "GitHub CLI not found. Installing..."
        winget install -e --id GitHub.cli --accept-source-agreements --accept-package-agreements
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
    }

    if (-not (Test-Command "gh")) {
        Write-Error "Failed to install GitHub CLI"
        return $false
    }

    Write-Success "GitHub CLI is available"

    # Check if copilot extension is installed
    $extensions = gh extension list 2>$null
    if ($extensions -match "copilot") {
        Write-Success "GitHub Copilot extension is already installed"
        return $true
    }

    Write-Info "Installing GitHub Copilot extension..."
    try {
        gh extension install github/gh-copilot
        Write-Success "GitHub Copilot extension installed"
        Write-Info "Run 'gh auth login' to authenticate with GitHub"
        return $true
    } catch {
        Write-Error "Failed to install Copilot extension: $_"
        return $false
    }
}

function Show-ApiKeyInstructions {
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Yellow
    Write-Host "API KEY CONFIGURATION" -ForegroundColor Yellow
    Write-Host "============================================" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Set the following environment variables:" -ForegroundColor White
    Write-Host ""
    Write-Host "For Claude:" -ForegroundColor Cyan
    Write-Host '  $env:ANTHROPIC_API_KEY = "your-api-key"'
    Write-Host '  [Environment]::SetEnvironmentVariable("ANTHROPIC_API_KEY", "your-key", "User")'
    Write-Host ""
    Write-Host "For OpenAI (Codex):" -ForegroundColor Cyan
    Write-Host '  $env:OPENAI_API_KEY = "your-api-key"'
    Write-Host '  [Environment]::SetEnvironmentVariable("OPENAI_API_KEY", "your-key", "User")'
    Write-Host ""
    Write-Host "For Google (Gemini):" -ForegroundColor Cyan
    Write-Host '  $env:GOOGLE_API_KEY = "your-api-key"'
    Write-Host '  [Environment]::SetEnvironmentVariable("GOOGLE_API_KEY", "your-key", "User")'
    Write-Host ""
    Write-Host "For GitHub Copilot:" -ForegroundColor Cyan
    Write-Host "  Run: gh auth login"
    Write-Host ""
}

# Main execution
Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "Polyglot-AI Tools Installer for Windows" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# If no specific tools selected, install all
if (-not ($Claude -or $Gemini -or $Codex -or $Copilot)) {
    $All = $true
}

# Check prerequisites
Write-Info "Checking prerequisites..."

if (-not (Test-Command "winget")) {
    Write-Warning "winget not found. Some installations may fail."
    Write-Info "Install App Installer from Microsoft Store for winget support."
}

# Install Node.js if needed (required for most tools)
if ($All -or $Claude -or $Gemini -or $Codex) {
    if (-not (Install-NodeIfNeeded)) {
        Write-Error "Node.js is required but could not be installed"
        exit 1
    }
}

$results = @{}

# Install selected tools
if ($All -or $Claude) {
    $results["Claude"] = Install-Claude
}

if ($All -or $Gemini) {
    $results["Gemini"] = Install-Gemini
}

if ($All -or $Codex) {
    $results["Codex"] = Install-Codex
}

if ($All -or $Copilot) {
    $results["Copilot"] = Install-Copilot
}

# Summary
Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host "Installation Summary" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan

foreach ($tool in $results.Keys) {
    if ($results[$tool]) {
        Write-Success "$tool : Installed"
    } else {
        Write-Warning "$tool : Not installed (manual setup may be required)"
    }
}

Show-ApiKeyInstructions

Write-Host "Installation complete!" -ForegroundColor Green
Write-Host ""
