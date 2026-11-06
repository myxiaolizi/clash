# =============================================================================
#  WebView2 稳态内存诊断
#
#  目的：区分"初始化峰值"与"稳态占用"，并按进程类型拆解。
#  perf-probe.ps1 做预算判定，本脚本只做细粒度诊断。
# =============================================================================

[CmdletBinding()]
param(
    [int]$WarmupSec = 12,
    [int]$Samples = 6,
    [int]$IntervalMs = 3000
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$Exe = Join-Path $Root 'src-tauri\target\release\clash-zhengl.exe'
if (-not (Test-Path $Exe)) {
    Write-Host "[SKIP] 未找到 $Exe" -ForegroundColor Yellow
    exit 2
}

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

Write-Host '=== WebView2 稳态诊断 ===' -ForegroundColor Cyan
$proc = Start-Process -FilePath $Exe -PassThru
Write-Host ("PID {0}，预热 {1} 秒等待 WebView2 完成初始化…" -f $proc.Id, $WarmupSec)
Start-Sleep -Seconds $WarmupSec

if ($proc.HasExited) {
    Write-Host '[FAIL] 进程已退出' -ForegroundColor Red
    exit 1
}

for ($i = 1; $i -le $Samples; $i++) {
    $proc.Refresh()
    $mainMb = $proc.WorkingSet64 / 1MB
    $mainPriv = $proc.PrivateMemorySize64 / 1MB
    $desc = Get-DescendantProcesses -RootPid $proc.Id

    $byType = @{}
    $childTotal = 0
    $childPriv = 0
    $totalPriv = 0
    foreach ($d in $desc) {
        $p = Get-Process -Id $d -ErrorAction SilentlyContinue
        if (-not $p) { continue }
        $childTotal += $p.WorkingSet64
        $childPriv += $p.PrivateMemorySize64
        $key = $p.ProcessName
        if (-not $byType.ContainsKey($key)) { $byType[$key] = @{ Count = 0; Bytes = 0 } }
        $byType[$key].Count++
        $byType[$key].Bytes += $p.WorkingSet64
    }

    Write-Host ''
    Write-Host ("[{0}] 主进程 WS {1:N1} MB | 子进程 {2} 个 WS 合计 {3:N1} MB | WS 总计 {4:N1} MB" -f `
        $i, $mainMb, $desc.Count, ($childTotal / 1MB), (($proc.WorkingSet64 + $childTotal) / 1MB))
    Write-Host ("       主进程 Private {0:N1} MB | 子进程 Private 合计 {1:N1} MB | Private 总计 {2:N1} MB" -f `
        $mainPriv, ($childPriv / 1MB), (($proc.PrivateMemorySize64 + $childPriv) / 1MB)) -ForegroundColor Yellow

    foreach ($k in ($byType.Keys | Sort-Object)) {
        $v = $byType[$k]
        Write-Host ("        {0,-24} x{1,-3} {2,8:N1} MB" -f $k, $v.Count, ($v.Bytes / 1MB)) -ForegroundColor DarkGray
    }

    if ($i -lt $Samples) { Start-Sleep -Milliseconds $IntervalMs }
}

try {
    if (-not $proc.HasExited) { $proc.Kill(); $proc.WaitForExit(3000) | Out-Null }
} catch { }
