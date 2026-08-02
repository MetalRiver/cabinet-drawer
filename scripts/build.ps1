# ============================================================
#  抽屉柜 - 生产构建脚本
#  用法：在 PowerShell 中执行  .\scripts\build.ps1
#  产物：src-tauri\target\release\bundle\{msi,nsis}\*.msi|*.exe
# ============================================================

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "=== 抽屉柜 - 生产构建 ===" -ForegroundColor Cyan
Write-Host ""

# 加载 MSVC 环境
$vcvars = $null
$candidates = @(
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
)
foreach ($p in $candidates) {
    if (Test-Path $p) { $vcvars = $p; break }
}
if ($vcvars) {
    Write-Host "[1/3] 加载 MSVC 环境..." -ForegroundColor Gray
    $envLines = cmd /c "`"$vcvars`" && set"
    foreach ($line in $envLines) {
        if ($line -match "^([^=]+)=(.*)$") {
            [System.Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
        }
    }
} else {
    Write-Host "[1/3] 未找到 vcvars64.bat" -ForegroundColor Yellow
}

if (-not (Test-Path "node_modules")) {
    Write-Host "[2/3] 安装 npm 依赖..." -ForegroundColor Gray
    npm install
} else {
    Write-Host "[2/3] node_modules 已存在" -ForegroundColor Gray
}

Write-Host "[3/3] 构建 release 包（首次 5-15 分钟）..." -ForegroundColor Green
Write-Host ""
npm run tauri build

Write-Host ""
Write-Host "=== 构建完成 ===" -ForegroundColor Cyan
$bundleDir = "src-tauri\target\release\bundle"
if (Test-Path $bundleDir) {
    Get-ChildItem $bundleDir -Recurse -Include *.msi,*.exe | ForEach-Object {
        $size = "{0:N1} MB" -f ($_.Length / 1MB)
        Write-Host "  $size   $($_.FullName)" -ForegroundColor Green
    }
}
