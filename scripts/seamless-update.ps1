#
# Seamless Update Script for Polyglot-AI Server (Windows)
# Performs zero-downtime updates using graceful shutdown
#
# Made by Tugcan Topaloglu
#

param(
    [switch]$Binary,
    [switch]$Docker,
    [switch]$SkipPull,
    [int]$Timeout = 30,
    [switch]$Rollback,
    [switch]$Help
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot = Split-Path -Parent $ScriptDir
$DockerDir = Join-Path $ProjectRoot "docker"

function Write-ColorOutput {
    param([string]$Message, [string]$Color = "White")
    Write-Host $Message -ForegroundColor $Color
}

function Log-Info { Write-ColorOutput "[INFO] $args" "Cyan" }
function Log-Success { Write-ColorOutput "[SUCCESS] $args" "Green" }
function Log-Warning { Write-ColorOutput "[WARNING] $args" "Yellow" }
function Log-Error { Write-ColorOutput "[ERROR] $args" "Red" }

function Print-Banner {
    Write-Host ""
    Write-ColorOutput "  ____       _             _       _        _    ___ " "Cyan"
    Write-ColorOutput " |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|" "Cyan"
    Write-ColorOutput " | |_) / _ \| | | | |/ _`` | |/ _ \| __|   / _ \  | | " "Cyan"
    Write-ColorOutput " |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | " "Cyan"
    Write-ColorOutput " |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|" "Cyan"
    Write-ColorOutput "               |___/ |___/                           " "Cyan"
    Write-Host ""
    Write-Host "       Seamless Update Script - Zero Downtime"
    Write-Host ""
}

function Test-Docker {
    try {
        $null = docker info 2>&1
        return $true
    } catch {
        return $false
    }
}

function Get-CurrentVersion {
    try {
        $result = docker inspect polyglot-server --format '{{.Config.Labels.version}}' 2>&1
        if ($LASTEXITCODE -eq 0) { return $result }
        return "not running"
    } catch {
        return "not running"
    }
}

function Pull-Latest {
    Log-Info "Pulling latest changes..."
    Push-Location $ProjectRoot
    try {
        git fetch origin main
        git pull origin main
        Log-Success "Code updated"
    } finally {
        Pop-Location
    }
}

function Build-NewImage {
    Log-Info "Building new server image..."
    Push-Location $DockerDir
    try {
        docker compose build --no-cache polyglot-server
        Log-Success "New image built successfully"
    } finally {
        Pop-Location
    }
}

function Invoke-GracefulShutdown {
    param([int]$TimeoutSeconds = 30)
    
    Log-Info "Initiating graceful shutdown (${TimeoutSeconds}s timeout)..."
    
    $running = docker ps --format '{{.Names}}' | Select-String "polyglot-server"
    if ($running) {
        # Send SIGTERM for graceful shutdown
        docker kill --signal=SIGTERM polyglot-server 2>$null
        
        $count = 0
        while ($count -lt $TimeoutSeconds) {
            $stillRunning = docker ps --format '{{.Names}}' | Select-String "polyglot-server"
            if (-not $stillRunning) { break }
            
            Start-Sleep -Seconds 1
            $count++
            Write-Host "`r   Waiting for connections to drain... $count/${TimeoutSeconds}s" -NoNewline
        }
        Write-Host ""
        
        $stillRunning = docker ps --format '{{.Names}}' | Select-String "polyglot-server"
        if ($stillRunning) {
            Log-Warning "Graceful shutdown timeout, forcing stop..."
            docker stop polyglot-server
        }
    }
}

function Start-NewContainer {
    Log-Info "Starting new server container..."
    Push-Location $DockerDir
    try {
        docker compose up -d polyglot-server
        
        Log-Info "Waiting for server to become healthy..."
        $count = 0
        $maxWait = 60
        
        while ($count -lt $maxWait) {
            try {
                $health = docker inspect --format='{{.State.Health.Status}}' polyglot-server 2>&1
            } catch {
                $health = "starting"
            }
            
            if ($health -eq "healthy") {
                Log-Success "Server is healthy and ready!"
                return $true
            }
            
            Start-Sleep -Seconds 2
            $count += 2
            Write-Host "`r   Health check... $count/${maxWait}s (status: $health)" -NoNewline
        }
        Write-Host ""
        Log-Error "Server failed to become healthy within ${maxWait}s"
        return $false
    } finally {
        Pop-Location
    }
}

function Invoke-Rollback {
    Log-Warning "Rolling back to previous version..."
    Push-Location $DockerDir
    try {
        $prevImage = docker images --format "{{.Repository}}:{{.Tag}}" | Select-String "polyglot-server" | Select-Object -First 2 | Select-Object -Last 1
        
        if ($prevImage) {
            docker tag $prevImage.ToString() polyglot-server:rollback
            docker compose up -d polyglot-server
            Log-Success "Rollback completed"
        } else {
            Log-Error "No previous image found for rollback"
            exit 1
        }
    } finally {
        Pop-Location
    }
}

function Update-Binary {
    Log-Info "Updating binary installation..."
    Push-Location $ProjectRoot
    try {
        cargo build --release
        
        $targetDir = "$env:USERPROFILE\.cargo\bin"
        
        # Backup current binaries
        if (Test-Path "$targetDir\polyglot-server.exe") {
            Copy-Item "$targetDir\polyglot-server.exe" "$targetDir\polyglot-server.exe.bak" -Force
        }
        
        # Install new binaries
        Copy-Item "target\release\polyglot-server.exe" $targetDir -Force
        Copy-Item "target\release\polyglot.exe" $targetDir -Force
        Copy-Item "target\release\polyglot-local.exe" $targetDir -Force
        
        Log-Success "Binaries updated to $targetDir"
    } finally {
        Pop-Location
    }
}

function Restart-WindowsService {
    $serviceName = "PolyglotServer"
    $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    
    if ($service) {
        Log-Info "Restarting Windows service..."
        Restart-Service -Name $serviceName
        Log-Success "Service restarted"
    } else {
        Log-Warning "Windows service not found. Start the server manually."
    }
}

function Show-Help {
    Write-Host "Usage: .\seamless-update.ps1 [OPTIONS]"
    Write-Host ""
    Write-Host "Options:"
    Write-Host "  -Docker       Update Docker container (default)"
    Write-Host "  -Binary       Update binary installation"
    Write-Host "  -SkipPull     Skip git pull"
    Write-Host "  -Timeout N    Graceful shutdown timeout in seconds (default: 30)"
    Write-Host "  -Rollback     Rollback to previous version"
    Write-Host "  -Help         Show this help"
}

# Main execution
Print-Banner

if ($Help) {
    Show-Help
    exit 0
}

if ($Rollback) {
    Invoke-Rollback
    exit 0
}

$updateType = if ($Binary) { "binary" } else { "docker" }

$currentVersion = Get-CurrentVersion
Log-Info "Current version: $currentVersion"

if (-not $SkipPull) {
    Pull-Latest
}

if ($updateType -eq "docker") {
    if (-not (Test-Docker)) {
        Log-Error "Docker is not running"
        exit 1
    }
    
    Build-NewImage
    Invoke-GracefulShutdown -TimeoutSeconds $Timeout
    
    if (Start-NewContainer) {
        Log-Success "Update completed successfully!"
    } else {
        Log-Error "Update failed, initiating rollback..."
        Invoke-Rollback
        exit 1
    }
} else {
    Update-Binary
    Restart-WindowsService
    Log-Success "Binary update completed!"
}

$newVersion = Get-CurrentVersion
Log-Info "New version: $newVersion"
