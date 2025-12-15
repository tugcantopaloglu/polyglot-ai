<#
.SYNOPSIS
    Polyglot-AI one-line installer (Windows PowerShell)

.DESCRIPTION
    Downloads published binaries from GitHub Releases, installs or updates them in place,
    and optionally installs AI CLI tools. No prompts; suitable for PowerShell or cmd one-liners.

.EXAMPLE
    # PowerShell
    irm https://raw.githubusercontent.com/tugcantopaloglu/polyglot-ai/main/scripts/install.ps1 | iex

    # cmd.exe
    powershell -NoProfile -ExecutionPolicy Bypass -Command "iwr https://raw.githubusercontent.com/tugcantopaloglu/polyglot-ai/main/scripts/install.ps1 -UseBasicParsing | iex"

    # Install AI tools too
    $env:POLYGLOT_WITH_TOOLS = "1"; irm https://raw.githubusercontent.com/tugcantopaloglu/polyglot-ai/main/scripts/install.ps1 | iex

    # Force reinstall
    $env:POLYGLOT_FORCE = "1"; irm https://raw.githubusercontent.com/tugcantopaloglu/polyglot-ai/main/scripts/install.ps1 | iex
#>

$WithTools = $env:POLYGLOT_WITH_TOOLS -eq "1"
$InstallDir = if ($env:POLYGLOT_INSTALL_DIR) { $env:POLYGLOT_INSTALL_DIR } else { "$env:USERPROFILE\.polyglot-ai" }
$Version = if ($env:POLYGLOT_VERSION) { $env:POLYGLOT_VERSION } else { "latest" }
$Force = $env:POLYGLOT_FORCE -eq "1"

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = "tugcantopaloglu/polyglot-ai"
$BinDir = Join-Path $InstallDir "bin"

function Write-Banner {
    Write-Host ""
    Write-Host "  ____       _             _       _        _    ___ " -ForegroundColor Cyan
    Write-Host " |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|" -ForegroundColor Cyan
    Write-Host " | |_) / _ \| | | | |/ _` | |/ _ \| __|   / _ \  | | " -ForegroundColor Cyan
    Write-Host " |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | " -ForegroundColor Cyan
    Write-Host " |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|" -ForegroundColor Cyan
    Write-Host "               |___/ |___/                           " -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Polyglot-AI Installer (GitHub Releases)" -ForegroundColor Green
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

function Write-WarningMessage {
    param([string]$Message)
    Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Failure {
    param([string]$Message)
    Write-Host "[ERROR] " -ForegroundColor Red -NoNewline
    Write-Host $Message
}

function Test-Command {
    param([string]$Command)
    $null = Get-Command $Command -ErrorAction SilentlyContinue
    return $?
}
function Normalize-Version {
    param([string]$Value)
    if (-not $Value) { return $null }
    return $Value.Trim().TrimStart("v")
}

function Get-TargetTriple {
    $arch = $env:PROCESSOR_ARCHITECTURE
    
    if (-not $arch) {
        try {
            $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        } catch {
            $arch = "AMD64"
        }
    }

    $archNormalized = switch ($arch) {
        "AMD64" { "X64" }
        "x64" { "X64" }
        "X64" { "X64" }
        "ARM64" { "Arm64" }
        "Arm64" { "Arm64" }
        "x86" { "X86" }
        "X86" { "X86" }
        default { $arch }
    }

    $isWindows = $true
    try {
        if ($PSVersionTable.PSVersion.Major -ge 6) {
            $isWindows = $IsWindows
        } elseif ($env:OS -eq "Windows_NT") {
            $isWindows = $true
        }
    } catch {
        $isWindows = ($env:OS -eq "Windows_NT")
    }

    if ($isWindows) {
        switch ($archNormalized) {
            "X64" { return @{ triple = "x86_64-pc-windows-msvc"; ext = ".exe" } }
            "Arm64" { throw "Windows Arm64 binaries are not published yet. Please use x64." }
            "X86" { throw "Windows x86 (32-bit) is not supported. Please use 64-bit Windows." }
            default { throw "Unsupported Windows architecture: $arch (normalized: $archNormalized)" }
        }
    }

    try {
        if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Linux)) {
            switch ($archNormalized) {
                "X64" { return @{ triple = "x86_64-unknown-linux-gnu"; ext = "" } }
                "Arm64" { return @{ triple = "aarch64-unknown-linux-gnu"; ext = "" } }
                default { throw "Unsupported Linux architecture: $arch" }
            }
        }

        if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
            switch ($archNormalized) {
                "X64" { return @{ triple = "x86_64-apple-darwin"; ext = "" } }
                "Arm64" { return @{ triple = "aarch64-apple-darwin"; ext = "" } }
                default { throw "Unsupported macOS architecture: $arch" }
            }
        }
    } catch {
        if ($archNormalized -eq "X64") {
            return @{ triple = "x86_64-pc-windows-msvc"; ext = ".exe" }
        }
    }

    throw "Unsupported platform/architecture combination: $arch"
}

function Get-ReleaseTag {
    param([string]$Requested)

    if ($Requested -and $Requested -ne "latest") {
        return $Requested
    }

    $url = "https://api.github.com/repos/$Repo/releases/latest"
    Write-Info "Fetching latest release tag..."

    try {
        $release = Invoke-RestMethod -Uri $url -Headers @{ "User-Agent" = "polyglot-ai-installer" }
        if ($release.tag_name) {
            Write-Success "Latest release: $($release.tag_name)"
            return $release.tag_name
        }
    } catch {
        Write-Failure "Could not determine latest release automatically."
    }

    throw "Specify a version via -Version or set POLYGLOT_VERSION."
}

function Get-InstalledVersion {
    param([string]$BinaryPath)

    if (-not (Test-Path $BinaryPath)) { return $null }

    try {
        $output = & $BinaryPath --version 2>$null
        $match = [regex]::Match($output, "([0-9]+\.[0-9]+\.[0-9]+)")
        if ($match.Success) { return $match.Groups[1].Value }
    } catch {
        return $null
    }

    return $null
}

function Download-And-InstallBinary {
    param(
        [string]$Name,
        [string]$Tag,
        [string]$Triple,
        [string]$Extension
    )

    $asset = "$Name-$Triple$Extension"
    $url = "https://github.com/$Repo/releases/download/$Tag/$asset"
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) $asset
    $destination = Join-Path $BinDir ("$Name$Extension")

    Write-Info "Downloading $asset ($Tag)..."
    Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing

    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
    Copy-Item $tmp $destination -Force
    Remove-Item $tmp -Force

    if (-not $Extension) {
        try { chmod +x $destination 2>$null } catch { }
    }

    Write-Success "$Name installed to $destination"
}

function Ensure-Path {
    Write-Info "Ensuring $BinDir is on PATH..."

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -notlike "*$BinDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$BinDir", "User")
        $env:Path += ";$BinDir"
        Write-Success "Added $BinDir to PATH (User scope)"
    } else {
        Write-Success "PATH already contains $BinDir"
    }
}
function Install-NodeJS {
    if (Test-Command "node") {
        $nodeVersion = node --version
        Write-Success "Node.js already installed: $nodeVersion"
        return
    }

    Write-Info "Installing Node.js (required for AI CLI tools)..."

    if (Test-Command "winget") {
        winget install -e --id OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
        $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
                    [System.Environment]::GetEnvironmentVariable("Path", "User")
    } else {
        $nodeInstaller = Join-Path $env:TEMP "node-installer.msi"
        Invoke-WebRequest -Uri "https://nodejs.org/dist/v20.10.0/node-v20.10.0-x64.msi" -OutFile $nodeInstaller
        Start-Process -FilePath "msiexec.exe" -ArgumentList "/i", $nodeInstaller, "/quiet", "/norestart" -Wait
    }

    if (Test-Command "node") {
        Write-Success "Node.js installed successfully"
    } else {
        Write-WarningMessage "Node.js installation may have failed. AI tools might not work."
    }
}

function Install-AITools {
    if (-not $WithTools) {
        return
    }

    Write-Info "Installing AI CLI tools..."

    if (-not (Test-Command "npm")) {
        Write-WarningMessage "npm not found. Skipping AI tool installation."
        return
    }

    try { npm install -g @anthropic-ai/claude-code 2>$null; Write-Success "Claude Code installed" } catch { Write-WarningMessage "Claude Code installation failed" }
    try { npm install -g @google/gemini-cli 2>$null; Write-Success "Gemini CLI installed" } catch { Write-WarningMessage "Gemini CLI installation failed" }
    try { npm install -g @openai/codex-cli 2>$null; Write-Success "Codex CLI installed" } catch { Write-WarningMessage "Codex CLI installation failed" }

    if (Test-Command "gh") {
        try { gh extension install github/gh-copilot 2>$null; Write-Success "GitHub Copilot installed" } catch { Write-WarningMessage "Copilot installation failed" }
    } else {
        Write-WarningMessage "GitHub CLI (gh) not found. Skipping Copilot installation."
    }
}
function Verify-Installation {
    Write-Info "Verifying installation..."

    $binary = Join-Path $BinDir "polyglot-local.exe"
    if (-not (Test-Path $binary)) {
        throw "polyglot-local was not found at $binary"
    }

    $version = Get-InstalledVersion $binary
    if ($version) {
        Write-Success "Installed version: $version"
    } else {
        Write-WarningMessage "Could not read version from polyglot-local --version"
    }
}

function Print-Success {
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Green
    Write-Host "   Installation Complete!" -ForegroundColor Green
    Write-Host "============================================" -ForegroundColor Green
    Write-Host ""
    Write-Host "Binaries installed to: $BinDir" -ForegroundColor White
    Write-Host ""
    Write-Host "Try: polyglot-local" -ForegroundColor Cyan
    Write-Host "Or:  polyglot-local ask \"Write hello world in Python\"" -ForegroundColor Cyan
    Write-Host ""
}

function Main {
    try {
        Write-Banner

        $target = Get-TargetTriple
        $tag = Get-ReleaseTag -Requested $Version
        $desiredVersion = Normalize-Version $tag

        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        New-Item -ItemType Directory -Path $BinDir -Force | Out-Null

        $binaries = @("polyglot-local", "polyglot", "polyglot-server")
        foreach ($name in $binaries) {
            $dest = Join-Path $BinDir ($name + $target.ext)
            $existing = Get-InstalledVersion $dest

            if (-not $Force -and $existing -and $desiredVersion -and ($existing -eq $desiredVersion)) {
                Write-Info "$name already at $existing, skipping download."
                continue
            }

            Download-And-InstallBinary -Name $name -Tag $tag -Triple $target.triple -Extension $target.ext
        }

        Ensure-Path

        if ($WithTools) {
            Install-NodeJS
            Install-AITools
        }

        Verify-Installation
        Print-Success
    } catch {
        Write-Host ""
        Write-Failure $_.Exception.Message
        Write-Host ""
        Write-Host "Installation failed. Please file an issue at https://github.com/$Repo/issues" -ForegroundColor Red
        exit 1
    }
}

Main
