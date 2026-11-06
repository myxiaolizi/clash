# =============================================================================
#  Clash Zhengl — 性能实测（对照契约 C.11 预算）
#
#  预算：
#    冷启动      ≤ 1.2 s
#    常驻 RSS    ≤ 90 MB
#    空闲 CPU    ≤ 1.5 %
#    安装包      ≤ 12 MB
#
#  用法：
#    powershell -ExecutionPolicy Bypass -File .\scripts\perf-probe.ps1
#    powershell -ExecutionPolicy Bypass -File .\scripts\perf-probe.ps1 -Samples 20
# =============================================================================

[CmdletBinding()]
param(
    [int]$Samples = 10,
    [int]$IntervalMs = 1000,
    [switch]$Json
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot

$ExeCandidates = @(
    (Join-Path $Root 'src-tauri\target\release\clash-zhengl.exe'),
    (Join-Path $Root 'src-tauri\target\release\clash_zhengl.exe')
)

$Exe = $ExeCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $Exe) {
    Write-Host '[SKIP] 未找到 release 可执行文件。请先运行：cd src-tauri; cargo build --release' -ForegroundColor Yellow
    exit 2
}

Write-Host ''
Write-Host '=== Clash Zhengl 性能实测 ===' -ForegroundColor Cyan
Write-Host "可执行文件: $Exe"
$exeSize = (Get-Item -LiteralPath $Exe).Length
Write-Host ('二进制大小: {0:N2} MB' -f ($exeSize / 1MB))
Write-Host ''

# ---------------------------------------------------------------------------
#  1. 冷启动
# ---------------------------------------------------------------------------
# 说明：这里测的是"进程从 CreateProcess 到主窗口出现"。
# 用 WaitForInputIdle 作为近似（Win32 提供，WebView2 初始化完成后才 idle）。
$startTimes = @()
for ($i = 1; $i -le 3; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $Exe -PassThru -WindowStyle Minimized
    try {
        $idle = $p.WaitForInputIdle(15000)
    } catch {
        $idle = $false
    }
    $sw.Stop()
    if ($idle) {
        $startTimes += $sw.Elapsed.TotalMilliseconds
    }
    Start-Sleep -Milliseconds 400
    if (-not $p.HasExited) { $p.Kill() }
    $p.WaitForExit(3000) | Out-Null
    Start-Sleep -Milliseconds 300
}

if ($startTimes.Count -eq 0) {
    Write-Host '[WARN] 三次启动都未进入 idle 状态，冷启动数据不可用' -ForegroundColor Yellow
    $coldStartMs = $null
} else {
    $coldStartMs = ($startTimes | Measure-Object -Average).Average
    Write-Host ('冷启动: {0:N0} ms (样本 {1} 次: {2})' -f `
        $coldStartMs, $startTimes.Count, (($startTimes | ForEach-Object { '{0:N0}' -f $_ }) -join ', '))
}

# ---------------------------------------------------------------------------
#  2. 常驻内存 + CPU
# ---------------------------------------------------------------------------
$proc = Start-Process -FilePath $Exe -PassThru -WindowStyle Minimized
Start-Sleep -Seconds 3

if ($proc.HasExited) {
    Write-Host '[FAIL] 进程启动后立即退出，无法采样' -ForegroundColor Red
    exit 1
}

# CPU 采样的基准：读两次 TotalProcessorTime，差值 / 间隔 / 核数
$cores = [Environment]::ProcessorCount
$rssSamples = @()
$cpuSamples = @()

$prevCpu = $proc.TotalProcessorTime
for ($i = 0; $i -lt $Samples; $i++) {
    Start-Sleep -Milliseconds $IntervalMs
    try {
        $proc.Refresh()
        if ($proc.HasExited) { break }

        $rssSamples += $proc.WorkingSet64

        $curCpu = $proc.TotalProcessorTime
        $deltaMs = ($curCpu - $prevCpu).TotalMilliseconds
        $prevCpu = $curCpu
        $cpuPct = ($deltaMs / $IntervalMs) * 100.0 / $cores
        $cpuSamples += $cpuPct
    } catch {
        break
    }
}

# 只统计**本进程树的** WebView2 子进程。
# 早期版本按进程名累加全机器 msedgewebview2.exe，会把其它应用（VS Code、
# 各种 Electron 应用）的内存算进来，导致 RSS 假超标。
function Get-DescendantProcesses {
    param([int]$RootPid)

    $result = @()
    $queue = [System.Collections.Generic.Queue[int]]::new()
    $queue.Enqueue($RootPid)
    $seen = @{}

    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        if ($seen.ContainsKey($cur)) { continue }
        $seen[$cur] = $true

        $children = Get-CimInstance Win32_Process -Filter "ParentProcessId=$cur" -ErrorAction SilentlyContinue
        foreach ($c in $children) {
            $result += [int]$c.ProcessId
            $queue.Enqueue([int]$c.ProcessId)
        }
    }
    return $result
}

$childRss = 0
$childCount = 0
try {
    $descendants = Get-DescendantProcesses -RootPid $proc.Id
    foreach ($pid_ in $descendants) {
        $op = Get-Process -Id $pid_ -ErrorAction SilentlyContinue
        if ($op) {
            $childRss += $op.WorkingSet64
            $childCount++
        }
    }
} catch { }

try {
    if (-not $proc.HasExited) { $proc.Kill(); $proc.WaitForExit(3000) | Out-Null }
} catch { }

Write-Host ''
if ($rssSamples.Count -gt 0) {
    $rssAvg = ($rssSamples | Measure-Object -Average).Average
    $rssMax = ($rssSamples | Measure-Object -Maximum).Maximum
    Write-Host ('应用进程 RSS: 平均 {0:N1} MB / 峰值 {1:N1} MB (样本 {2})' -f `
        ($rssAvg / 1MB), ($rssMax / 1MB), $rssSamples.Count)
    if ($childRss -gt 0) {
        Write-Host ('WebView2 子进程({0} 个): {1:N1} MB  [信息性，契约预算不计入]' -f `
            $childCount, ($childRss / 1MB)) -ForegroundColor DarkGray
    }
} else {
    Write-Host '[WARN] 未采到内存样本' -ForegroundColor Yellow
    $rssMax = $null
}

if ($cpuSamples.Count -gt 0) {
    $cpuAvg = ($cpuSamples | Measure-Object -Average).Average
    $cpuMax = ($cpuSamples | Measure-Object -Maximum).Maximum
    Write-Host ('空闲 CPU: 平均 {0:N2}% / 峰值 {1:N2}% ({2} 核归一化)' -f $cpuAvg, $cpuMax, $cores)
} else {
    Write-Host '[WARN] 未采到 CPU 样本' -ForegroundColor Yellow
    $cpuAvg = $null
}

# ---------------------------------------------------------------------------
#  3. 安装包
# ---------------------------------------------------------------------------
Write-Host ''
$setupCandidates = @(
    (Join-Path $Root 'src-tauri\target\release\bundle\nsis\Clash Zhengl_0.1.0_x64-setup.exe'),
    (Join-Path $Root 'src-tauri\target\release\bundle\nsis\ClashZhengl_0.1.0_x64-setup.exe')
)
$setup = $setupCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if ($setup) {
    $setupMb = (Get-Item -LiteralPath $setup).Length / 1MB
    Write-Host ('安装包: {0:N2} MB  ({1})' -f $setupMb, (Split-Path -Leaf $setup))
} else {
    $setupMb = $null
    Write-Host '安装包: 未构建（cargo tauri build 后可测）' -ForegroundColor DarkGray
}

# ---------------------------------------------------------------------------
#  4. 对照预算
# ---------------------------------------------------------------------------
$budget = @{
    coldStartMs = 1200
    rssMb       = 90
    cpuPct      = 1.5
    setupMb     = 12
}

$checks = @()
function Add-Check {
    param($Name, $Actual, $Limit, $Unit, $Fmt)
    $status = 'N/A'
    if ($null -ne $Actual) {
        if ($Actual -le $Limit) { $status = 'PASS' } else { $status = 'FAIL' }
    }
    $a = if ($null -ne $Actual) { $Fmt -f $Actual } else { '—' }
    $script:checks += [pscustomobject]@{
        '指标'   = $Name
        '实测'   = $a
        '预算'   = "$Limit $Unit"
        '结果'   = $status
    }
}

Add-Check '冷启动'      $coldStartMs $budget.coldStartMs 'ms' '{0:N0}'
Add-Check '应用 RSS'    $(if ($rssMax) { $rssMax / 1MB } else { $null }) $budget.rssMb 'MB' '{0:N1}'
Add-Check '空闲 CPU'    $cpuAvg      $budget.cpuPct     '%'  '{0:N2}'
Add-Check '安装包'      $setupMb     $budget.setupMb    'MB' '{0:N2}'

Write-Host ''
Write-Host '--- 预算对照（契约 C.11）---' -ForegroundColor Cyan
$checks | Format-Table -AutoSize

$fails = @($checks | Where-Object { $_.'结果' -eq 'FAIL' })
Write-Host ''
if ($fails.Count -eq 0) {
    Write-Host '全部指标达标' -ForegroundColor Green
    exit 0
} else {
    Write-Host ("{0} 项超标" -f $fails.Count) -ForegroundColor Red
    exit 1
}
