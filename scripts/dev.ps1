# ============================================================
#  Drawer Box - Dev Mode Launcher
#  Usage: powershell -ExecutionPolicy Bypass -File .\scripts\dev.ps1
#  Auto-loads MSVC env, installs deps if missing, then runs Tauri dev.
# ============================================================

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host ""
Write-Host "=== Drawer Box - Dev Mode ===" -ForegroundColor Cyan
Write-Host ""

# Load MSVC env
$vcvars = $null
$msvcPaths = @(
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
)
foreach ($p in $msvcPaths) {
    if (Test-Path $p) { $vcvars = $p; break }
}
if ($vcvars) {
    Write-Host "[1/3] Loading MSVC env..." -ForegroundColor Gray
    Push-Location (Split-Path $vcvars)
    cmd /c "vcvars64.bat && set" | ForEach-Object {
        if ($_ -match "^([^=]+)=(.*)$") {
            [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
    Pop-Location
} else {
    Write-Host "[1/3] MSVC vcvars64.bat not found, build may fail." -ForegroundColor Yellow
}

# Install npm deps
if (-not (Test-Path "node_modules")) {
    Write-Host "[2/3] Installing npm dependencies..." -ForegroundColor Gray
    npm install
} else {
    Write-Host "[2/3] node_modules ready" -ForegroundColor Gray
}

# Launch
Write-Host "[3/3] Starting Tauri dev (close window to stop)..." -ForegroundColor Green
Write-Host ""
npm run tauri dev
