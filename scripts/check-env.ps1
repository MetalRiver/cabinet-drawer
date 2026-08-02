# ============================================================
#  Drawer Box - Environment Check
#  Usage: powershell -ExecutionPolicy Bypass -File .\scripts\check-env.ps1
# ============================================================

$ok = $true

function Show-OK {
    param([string]$name, [string]$version)
    $line = "  [OK]  {0,-14}{1}" -f $name, $version
    Write-Host $line -ForegroundColor Green
}

function Show-Warn {
    param([string]$name, [string]$hint = "")
    $line = "  [!!]  {0,-14}not installed" -f $name
    Write-Host $line -ForegroundColor Red
    if ($hint) { Write-Host ("        " + $hint) -ForegroundColor Yellow }
    $script:ok = $false
}

function Show-Skip {
    param([string]$name, [string]$note)
    $line = "  [--]  {0,-14}{1}" -f $name, $note
    Write-Host $line -ForegroundColor Yellow
}

Write-Host ""
Write-Host "=== Drawer Box - Environment Check ===" -ForegroundColor Cyan
Write-Host ""

# Node
$c = Get-Command node -ErrorAction SilentlyContinue
if ($c) { Show-OK "Node.js" $c.Version.ToString() } else { Show-Warn "Node.js" "Download: https://nodejs.org/  (need v18+)" }

# npm
$c = Get-Command npm -ErrorAction SilentlyContinue
if ($c) { $v = & npm --version 2>$null; Show-OK "npm" $v } else { Show-Warn "npm" "" }

# Cargo
$c = Get-Command cargo -ErrorAction SilentlyContinue
if ($c) { $v = & cargo --version 2>$null; Show-OK "Cargo" $v } else { Show-Warn "Cargo" "Install: https://rustup.rs/" }

# rustc
$c = Get-Command rustc -ErrorAction SilentlyContinue
if ($c) { $v = & rustc --version 2>$null; Show-OK "rustc" $v } else { Show-Warn "rustc" "" }

# MSVC (check vcvars64.bat)
$msvcFound = $false
$msvcPaths = @(
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
)
foreach ($p in $msvcPaths) {
    if (Test-Path $p) { Show-OK "MSVC" $p; $msvcFound = $true; break }
}
if (-not $msvcFound) {
    Show-Warn "MSVC" "Open Visual Studio Installer -> Modify -> check 'Desktop development with C++'"
}

# WebView2 Runtime
$wv2Out = reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" /v pv 2>&1 | Out-String
$wv2ver = ""
if ($wv2Out -match "REG_SZ\s+(\S+)") { $wv2ver = $Matches[1] }
if ($wv2ver) { Show-OK "WebView2" $wv2ver } else { Show-Skip "WebView2" "(auto-installed by MSI)" }

Write-Host ""
if ($ok) {
    Write-Host "READY -> run .\scripts\dev.ps1 to start dev mode" -ForegroundColor Green
} else {
    Write-Host "NOT READY -> fix the items marked [!!] above" -ForegroundColor Yellow
}
Write-Host ""
