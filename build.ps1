# 构建并安装 mos_打光（EffectCraft 外部效果辅助程序 v1.0.0）
# 用法：在目录内执行  .\build.ps1

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$appData = $env:APPDATA
if (-not $appData) { $appData = [Environment]::GetFolderPath('ApplicationData') }

Write-Host '[1/4] 激活 Rust 工具链 ...'
if (Test-Path 'D:\ruanjian\rust-env\activate.ps1') {
    . 'D:\ruanjian\rust-env\activate.ps1'
    $env:CARGO_NET_OFFLINE = 'true'
} else {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) { throw '找不到 cargo，也未发现 D:\ruanjian\rust-env\activate.ps1' }
    Write-Host "    使用系统 $($cargo.Source)"
}

Write-Host '[2/4] 编译 helper ...'
Push-Location (Join-Path $here 'helper')
$targetDir = Join-Path $env:TEMP 'mos-rimlight-target'
try {
    & cargo build --release --target-dir $targetDir
    if ($LASTEXITCODE -ne 0) { throw "编译失败（cargo 退出码 $LASTEXITCODE），没有安装任何文件" }
} finally {
    Pop-Location
}

$exe = Join-Path $targetDir 'release\effectcraft-helper-mos-rimlight.exe'
if (-not (Test-Path $exe)) { throw "编译失败：找不到 $exe" }

Write-Host '[3/4] 安装 helper 到 dist 和 EffectCraft ...'
Get-Process effectcraft-helper-mos-rimlight -ErrorAction SilentlyContinue | ForEach-Object {
    $_.Kill()
    $_.WaitForExit()
}

$distDir = Join-Path $here 'dist'
New-Item -ItemType Directory -Force -Path $distDir | Out-Null
Copy-Item $exe (Join-Path $distDir 'effectcraft-helper-mos-rimlight.exe') -Force

$helpersDir = Join-Path $appData 'EffectCraft\helpers'
New-Item -ItemType Directory -Force -Path $helpersDir | Out-Null
Copy-Item $exe (Join-Path $helpersDir 'effectcraft-helper-mos-rimlight.exe') -Force

Write-Host '[4/4] 确保开机自加载脚本存在 ...'
$startup = Join-Path $appData 'EffectCraft\Scripts\Startup'
New-Item -ItemType Directory -Force -Path $startup | Out-Null
Copy-Item (Join-Path $here 'tools\mos-load-helpers.jsx') (Join-Path $startup 'mos-load-helpers.jsx') -Force

Write-Host ''
Write-Host "完成：$helpersDir\effectcraft-helper-mos-rimlight.exe"
Write-Host '重启 EffectCraft 后，效果列表「MOS 视效」分类下会自动加载 mos_打光 v1.0.0。'
Write-Host '（当前已开着的窗口：执行一次 效果 ▸ 扫描效果辅助程序... 即可立刻加载）'
