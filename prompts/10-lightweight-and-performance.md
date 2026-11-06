# 10 · 轻量化与性能工程

> **定位**：本章是 `C-conventions.md §C.11`（v1.1）性能预算的**落地手册**——怎么量、怎么改、不达标怎么退。
> **铁律**：性能结论只能用**实测数字**表达。禁止出现「应该很轻」「理论上没问题」这类表述（01 章 §8）。
> **贯穿阶段**：P0 建测量桩 → P1/P2/P5 每阶段跑一次 → P6 集中调优 → P8 验收。

---

## 10.1 预算表（与 C.11 完全一致，不得改写数字）

| # | 指标 | 预算 | 测量方法 | 阶段门禁 |
| --- | --- | --- | --- | --- |
| M1 | 应用进程 RSS（窗口隐藏 + 内核运行） | **≤ 90 MB** | `perf-probe.ps1 -Mode idle`，采样 60s 取 P95 | P1 / P5 / P8 |
| M2 | 应用进程 RSS（窗口可见） | **≤ 140 MB** | `perf-probe.ps1 -Mode visible` | P5 / P8 |
| M3 | 空闲 CPU（5 分钟平均，单核占比） | **≤ 1.5%** | `perf-probe.ps1 -Mode idle -Minutes 5` | P3 / P8 |
| M4 | 测速期间 CPU | **≤ 25% 单核 且持续 ≤ 5s** | `perf-probe.ps1 -Mode speedtest` | P4 / P8 |
| M5 | 冷启动到托盘可用 | **≤ 1.2s** | `perf-probe.ps1 -Mode startup`，5 次取中位数 | P0 建桩 / P1 / P7 / P8 |
| M6 | 含内核代理可用 | **≤ 3s** | 同上，计时到 `core:state = Running` + 端口可连 | P1 / P8 |
| M7 | 安装包体积（不含内核/WebView2） | **≤ 12 MB** | 口径（C.11 v1.1）：**签名后的** `ClashZhengl_<ver>_x64-setup.exe`，`Get-Item .Length` | P7 / P8 |
| M8 | 常驻磁盘占用 | **≤ 120 MB** | 口径（C.11 v1.1）：安装目录 + `%LOCALAPPDATA%` 运行数据，**不含** `%APPDATA%` 里的订阅原文 | P7 / P8 |
| M9 | 后台唤醒 | 空闲时**无 < 30s** 的定时唤醒 | 见 10.3.5（ETW / 简易打点） | P3 / P8 |

> **C.11 v1.1 的仲裁规则（必须照做）**：实测落在预算的 **100%–120%** 之间时，允许保留并记入
> `docs/perf-log.md`，但**下一阶段必须给出改进项**；**超过 120% 视为不达标**，必须立即按 10.7 处置。
> 这不是放宽标准，而是给「差一点点」留一条有账可查的通道——记账，不是免测。

**预算分解（给自己留余量的工程口径）**：

| 组成 | 目标占用 | 说明 |
| --- | --- | --- |
| Rust 主进程基线（空 Tauri 窗口） | 8–14 MB | 超 25 MB 说明依赖或 WebView 配置有问题 |
| WebView2 渲染进程（隐藏时） | 0（可挂起） | 隐藏后 WebView2 仍可能占 20–40 MB，见 10.4.4 |
| WebView2 渲染进程（可见） | 35–60 MB | 不可控，属预算内固定成本 |
| 合并后配置常驻 | ≤ 3 MB | 需 10.4.5 的「不常驻」策略 |
| 运行态（节点结果、历史） | ≤ 4 MB | 节点数 500 时的量级 |
| 余量 | 其余 | **不要把余量花光** |

---

## 10.2 测量前置条件（不满足则数字无效）

| 项 | 要求 |
| --- | --- |
| 构建类型 | **必须 release**，`cargo tauri build --no-bundle` 或 NSIS 安装后的 exe；debug 数字无意义 |
| 系统 | Windows 11 x64 或 Win10 1809+，无其它重负载 |
| 采样工具 | `perf-probe.ps1`（下节给出完整实现，落 `scripts/perf-probe.ps1`，路径见 C.2） |
| 内核状态 | M1 要求内核**运行中**（这才是真实常驻态）；测「纯 UI」另记一组参考值 |
| 预热 | 启动后等待 30s 再开始采样，避开启动期 JIT/缓存 |
| 次数 | RSS/CPU 每组 3 次，取中位数；启动耗时 5 次取中位数 |
| 记录 | 结果贴进 PR 描述与 `docs/perf-baseline.md`，格式见 10.6 |

> **测量自身的干扰**：`Get-Counter` 自身开销约 0.3–1% CPU。测 M3 时优先用「进程 CPU 时间增量 ÷ 墙钟」，
> 不用 `Get-Counter '\Process(*)\% Processor Time'`。

---

## 10.3 `scripts/perf-probe.ps1` 完整实现

> 落盘路径：`scripts/perf-probe.ps1`（C.2 已登记）。pwsh 7 语法，无第三方模块依赖。

### 10.3.1 完整脚本

```powershell
<#
.SYNOPSIS
    Clash Zhengl 性能探针：采集 RSS / CPU / 启动耗时 / 包体 / 磁盘占用。
.EXAMPLE
    pwsh -File .\scripts\perf-probe.ps1 -Mode idle
    pwsh -File .\scripts\perf-probe.ps1 -Mode visible -Seconds 60
    pwsh -File .\scripts\perf-probe.ps1 -Mode speedtest
    pwsh -File .\scripts\perf-probe.ps1 -Mode startup -Runs 5
    pwsh -File .\scripts\perf-probe.ps1 -Mode disk
#>
[CmdletBinding()]
param(
    [ValidateSet('idle','visible','speedtest','startup','disk','all')]
    [string]$Mode = 'idle',
    [int]$Seconds = 60,
    [int]$Minutes = 5,
    [int]$Runs = 5,
    [string]$ProcessName = 'clash-zhengl',
    [string]$ExePath = "$env:ProgramFiles\Clash Zhengl\clash-zhengl.exe",
    [string]$OutJson  = ''
)

$ErrorActionPreference = 'Stop'
$script:results = [ordered]@{ mode = $Mode; at = (Get-Date).ToString('o'); samples = @() }

function Get-TargetProcess {
    param([string]$Name)
    $p = Get-Process -Name $Name -ErrorAction SilentlyContinue
    if (-not $p) { throw "进程 $Name 未运行。请先启动应用（$Mode 模式需要应用已运行）。" }
    return $p | Sort-Object WorkingSet64 -Descending | Select-Object -First 1
}

function Get-RssMB {
    param($p)
    return [math]::Round($p.WorkingSet64 / 1MB, 2)
}

function Get-WebViewRssMB {
    # WebView2 渲染进程：msedgewebview2.exe，父进程为本应用
    $mine = (Get-Process -Name $ProcessName -ErrorAction SilentlyContinue).Id
    $wv = Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" -ErrorAction SilentlyContinue |
          Where-Object { $mine -contains $_.ParentProcessId }
    $sum = 0
    foreach ($w in $wv) {
        $gp = Get-Process -Id $w.ProcessId -ErrorAction SilentlyContinue
        if ($gp) { $sum += $gp.WorkingSet64 }
    }
    return [math]::Round($sum / 1MB, 2)
}

function Get-Percentile {
    param([double[]]$Values, [double]$P)
    if (-not $Values -or $Values.Count -eq 0) { return $null }
    $sorted = $Values | Sort-Object
    $idx = [math]::Ceiling(($P / 100.0) * $sorted.Count) - 1
    if ($idx -lt 0) { $idx = 0 }
    return [math]::Round($sorted[$idx], 2)
}

function Assert-Budget {
    param([string]$Name, [double]$Value, [double]$Budget, [string]$Unit)
    $ok = $Value -le $Budget
    $tag = if ($ok) { 'PASS' } else { 'FAIL' }
    Write-Host ("{0,-6} {1,-34} {2,9} {3}  (预算 <= {4} {3})" -f $tag, $Name, $Value, $Unit, $Budget) `
        -ForegroundColor $(if ($ok) { 'Green' } else { 'Red' })
    return $ok
}

# ---------- M1 / M2：RSS 采样 ----------
function Invoke-RssProbe {
    param([int]$DurationSec, [string]$Label, [double]$BudgetMB)
    $p = Get-TargetProcess -Name $ProcessName
    Write-Host "采样 $Label：进程 $($p.Id) 共 $DurationSec 秒..." -ForegroundColor Cyan
    $rss = [System.Collections.Generic.List[double]]::new()
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $DurationSec) {
        $p.Refresh()
        $rss.Add((Get-RssMB $p))
        Start-Sleep -Milliseconds 1000
    }
    $sw.Stop()
    $wv = Get-WebViewRssMB
    $p95 = Get-Percentile -Values $rss.ToArray() -P 95
    $max = ($rss | Measure-Object -Maximum).Maximum
    Write-Host ("RSS 采样 n={0} P95={1} MB max={2} MB 主进程=当前{3} MB / WebView2={4} MB" -f `
        $rss.Count, $p95, [math]::Round($max,2), (Get-RssMB $p), $wv)
    $ok = Assert-Budget -Name "$Label RSS(P95)" -Value $p95 -Budget $BudgetMB -Unit 'MB'
    $script:results.samples += [ordered]@{
        kind='rss'; label=$Label; p95=$p95; max=[math]::Round($max,2); webview=$wv; budget=$BudgetMB; pass=$ok
    }
    return $ok
}

# ---------- M3：空闲 CPU（进程 CPU 时间增量 / 墙钟） ----------
function Invoke-IdleCpuProbe {
    param([int]$DurationMin)
    $p = Get-TargetProcess -Name $ProcessName
    $cores = [Environment]::ProcessorCount
    $p.Refresh(); $cpu0 = $p.TotalProcessorTime
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $secs = $DurationMin * 60
    Write-Host "采样空闲 CPU $Minute 分钟（共 $cores 逻辑核）..." -ForegroundColor Cyan
    while ($sw.Elapsed.TotalSeconds -lt $secs) {
        if ([Console]::KeyAvailable) { break }   # 允许 Ctrl+C 之外的手动中断
        Start-Sleep -Seconds 5
    }
    $sw.Stop()
    $p.Refresh()
    $cpuDelta = ($p.TotalProcessorTime - $cpu0).TotalSeconds
    # 单核占比 = CPU 时间增量 / 墙钟；再折算百分比
    $pct = [math]::Round(($cpuDelta / $sw.Elapsed.TotalSeconds) * 100, 2)
    Write-Host ("空闲 CPU：cpu_time={0:N2}s wall={1:N2}s 单核占比={2}%" -f $cpuDelta, $sw.Elapsed.TotalSeconds, $pct)
    $ok = Assert-Budget -Name '空闲 CPU(单核%)' -Value $pct -Budget 1.5 -Unit '%'
    $script:results.samples += [ordered]@{ kind='cpu'; pct=$pct; wall=[math]::Round($sw.Elapsed.TotalSeconds,2); budget=1.5; pass=$ok }
    return $ok
}

# ---------- M4：测速期间 CPU ----------
function Invoke-SpeedtestCpuProbe {
    $p = Get-TargetProcess -Name $ProcessName
    Write-Host "触发测速..." -ForegroundColor Cyan
    # 通过 UI 触发或 IPC；此处假定用户已手动触发，脚本只负责采样
    Write-Host "请在 3 秒内于应用内点击「立即测速」（或脚本调用 speedtest_start）"
    Start-Sleep -Seconds 3
    $p.Refresh(); $cpu0 = $p.TotalProcessorTime
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $peak = 0.0; $prevCpu = $cpu0; $prevT = 0.0
    while ($sw.Elapsed.TotalSeconds -lt 30) {
        Start-Sleep -Milliseconds 500
        $p.Refresh()
        $t = $sw.Elapsed.TotalSeconds
        $inst = (($p.TotalProcessorTime - $prevCpu).TotalSeconds / ($t - $prevT)) * 100
        if ($inst -gt $peak) { $peak = $inst }
        $prevCpu = $p.TotalProcessorTime; $prevT = $t
        if ($inst -lt 2 -and $t -gt 6) { break }   # 测速已结束
    }
    $sw.Stop()
    $peak = [math]::Round($peak, 2)
    Write-Host ("测速期 CPU 峰值={0}% 总时长={1:N1}s" -f $peak, $sw.Elapsed.TotalSeconds)
    $ok = ($peak -le 25)
    $ok = Assert-Budget -Name '测速期 CPU 峰值' -Value $peak -Budget 25 -Unit '%'
    $script:results.samples += [ordered]@{ kind='cpu-speedtest'; peak=$peak; elapsed=[math]::Round($sw.Elapsed.TotalSeconds,1); budget=25; pass=$ok }
    return $ok
}

# ---------- M5 / M6：启动耗时 ----------
function Invoke-StartupProbe {
    param([int]$Count)
    if (-not (Test-Path $ExePath)) { throw "找不到 $ExePath，请用 -ExePath 指定 release 版可执行文件。" }
    $times = @(); $proxyTimes = @()
    for ($i = 1; $i -le $Count; $i++) {
        Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Stop-Process -Force
        Start-Sleep -Milliseconds 800      # 等端口/互斥释放
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $proc = Start-Process -FilePath $ExePath -PassThru
        # 轮询托盘可用：以进程存在 + 主窗口句柄出现为准（隐藏启动时用 IPC 端口代替）
        $ready = $false
        while ($sw.Elapsed.TotalSeconds -lt 10) {
            $proc.Refresh()
            if ($proc.MainWindowHandle -ne 0) { $ready = $true; break }
            if ($proc.HasExited) { throw "启动即退出，请检查日志。" }
            Start-Sleep -Milliseconds 20
        }
        $sw.Stop()
        $ms = [math]::Round($sw.Elapsed.TotalMilliseconds, 0)
        $times += $ms
        Write-Host ("第 {0} 次：托盘可用 {1} ms{2}" -f $i, $ms, $(if (-not $ready) { '（超时，未检测到窗口）' } else { '' }))

        # M6：等内核就绪 —— 探测 mixed-port 是否可连（端口从设置读取或默认 10801）
        $sw2 = [System.Diagnostics.Stopwatch]::StartNew()
        $mixedPort = 10801
        while ($sw2.Elapsed.TotalSeconds -lt 15) {
            $probe = Test-NetConnection -ComputerName '127.0.0.1' -Port $mixedPort -InformationLevel Quiet -WarningAction SilentlyContinue
            if ($probe) { break }
            Start-Sleep -Milliseconds 50
        }
        $sw2.Stop()
        $proxyTimes += [math]::Round($sw2.Elapsed.TotalMilliseconds, 0)
        Write-Host ("        代理可用 {0} ms（端口 {1}）" -f $proxyTimes[-1], $mixedPort)
        Start-Sleep -Milliseconds 500
    }
    $median  = ($times | Sort-Object)[[int]($times.Count / 2)]
    $median2 = ($proxyTimes | Sort-Object)[[int]($proxyTimes.Count / 2)]
    Write-Host ("启动中位数：托盘 {0} ms / 代理可用 {1} ms；明细 {2}" -f $median, $median2, ($times -join ', '))
    $ok1 = Assert-Budget -Name '冷启动到托盘可用'      -Value $median  -Budget 1200 -Unit 'ms'
    $ok2 = Assert-Budget -Name '含内核代理可用'        -Value $median2 -Budget 3000 -Unit 'ms'
    $script:results.samples += [ordered]@{ kind='startup'; trayMedian=$median; proxyMedian=$median2; runs=$times; pass=($ok1 -and $ok2) }
    return ($ok1 -and $ok2)
}

# ---------- M7 / M8：包体与磁盘 ----------
# C.11 v1.1 口径：
#   M7 = 签名后的 ClashZhengl_<ver>_x64-setup.exe（不含内核/WebView2）
#   M8 = 安装目录 + %LOCALAPPDATA% 运行数据，**不含** %APPDATA% 里的订阅原文
function Invoke-DiskProbe {
    $installDir = "$env:ProgramFiles\Clash Zhengl"
    if (-not (Test-Path $installDir)) { $installDir = "$env:LOCALAPPDATA\Programs\Clash Zhengl" }
    $targets = @($installDir, "$env:LOCALAPPDATA\ClashZhengl")
    $total = 0
    foreach ($t in $targets) {
        if (Test-Path $t) {
            $size = (Get-ChildItem $t -Recurse -File -ErrorAction SilentlyContinue |
                     Measure-Object -Property Length -Sum).Sum
            $mb = [math]::Round($size / 1MB, 2)
            $total += $size
            Write-Host ("{0,-52} {1,8} MB" -f $t, $mb)
        } else {
            Write-Host ("{0,-52} {1,8}" -f $t, '(不存在)')
        }
    }
    # 仅报告不计数：%APPDATA% 里的订阅原文按 C.11 口径不计入 M8
    $roaming = "$env:APPDATA\ClashZhengl"
    if (Test-Path $roaming) {
        $rMB = [math]::Round(((Get-ChildItem $roaming -Recurse -File -ErrorAction SilentlyContinue |
                  Measure-Object -Property Length -Sum).Sum) / 1MB, 2)
        Write-Host ("{0,-52} {1,8} MB  (参考值，不计入 M8)" -f $roaming, $rMB)
    }
    $totalMB = [math]::Round($total / 1MB, 2)
    $ok = Assert-Budget -Name '常驻磁盘占用(M8 口径)' -Value $totalMB -Budget 120 -Unit 'MB'

    # 安装包体积（取最新一个 setup；若已签名，此为签名后体积）
    $setup = Get-ChildItem -Path 'src-tauri\target\release\bundle\nsis' -Filter '*_x64-setup.exe' `
                -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($setup) {
        $pkgMB = [math]::Round($setup.Length / 1MB, 2)
        Write-Host ("{0,-52} {1,8} MB  (M7 口径)" -f $setup.Name, $pkgMB)
        $okPkg = Assert-Budget -Name '安装包体积(M7 口径)' -Value $pkgMB -Budget 12 -Unit 'MB'
        $script:results.samples += [ordered]@{ kind='package'; name=$setup.Name; mb=$pkgMB; budget=12; pass=$okPkg }
        $ok = $ok -and $okPkg
    } else {
        Write-Host '未找到 NSIS 产物（先跑 cargo tauri build）' -ForegroundColor Yellow
    }
    $script:results.samples += [ordered]@{ kind='disk'; mb=$totalMB; budget=120; pass=$ok }
    return $ok
}

# ---------- 主流程 ----------
$allPass = $true
switch ($Mode) {
    'idle'      { $allPass = Invoke-RssProbe -DurationSec $Seconds -Label '隐藏+内核运行' -BudgetMB 90 }
    'visible'   { $allPass = Invoke-RssProbe -DurationSec $Seconds -Label '窗口可见' -BudgetMB 140 }
    'speedtest' { $allPass = Invoke-SpeedtestCpuProbe }
    'startup'   { $allPass = Invoke-StartupProbe -Count $Runs }
    'disk'      { $allPass = Invoke-DiskProbe }
    'all' {
        $r = @()
        $r += Invoke-StartupProbe -Count $Runs
        $r += Invoke-RssProbe -DurationSec $Seconds -Label '隐藏+内核运行' -BudgetMB 90
        $r += Invoke-IdleCpuProbe -DurationMin $Minutes
        $r += Invoke-DiskProbe
        $allPass = -not ($r -contains $false)
    }
}

$script:results.pass = $allPass
if ($OutJson) {
    $script:results | ConvertTo-Json -Depth 6 | Set-Content -Path $OutJson -Encoding utf8
    Write-Host "结果已写入 $OutJson"
}
if ($allPass) { Write-Host "`n结论：全部指标在预算内。" -ForegroundColor Green; exit 0 }
else          { Write-Host "`n结论：存在超预算指标，按 10.7 降级阶梯处置。" -ForegroundColor Red; exit 1 }
```

### 10.3.2 使用方式

```powershell
# 1) 先构建 release
cargo tauri build --no-bundle

# 2) 启动应用（隐藏到托盘 + 内核运行），再采样
pwsh -File .\scripts\perf-probe.ps1 -Mode idle -Seconds 60 -OutJson .\docs\perf\rss-idle.json

# 3) 窗口打开后再采
pwsh -File .\scripts\perf-probe.ps1 -Mode visible -Seconds 60

# 4) 空闲 CPU（5 分钟，期间不要动鼠标点界面）
pwsh -File .\scripts\perf-probe.ps1 -Mode idle -Minutes 5

# 5) 启动耗时（5 次中位数）
pwsh -File .\scripts\perf-probe.ps1 -Mode startup -Runs 5 -ExePath "src-tauri\target\release\clash-zhengl.exe"

# 6) 包体 + 磁盘
pwsh -File .\scripts\perf-probe.ps1 -Mode disk

# 7) 一把梭（约 8 分钟）
pwsh -File .\scripts\perf-probe.ps1 -Mode all -Minutes 5 -OutJson .\docs\perf\full.json
```

### 10.3.3 单指标快查（不依赖脚本时手工核对）

```powershell
# 主进程 RSS（当前值，MB）
(Get-Process clash-zhengl).WorkingSet64 / 1MB

# 按采样间隔取 60 个点算 P95
$all = 1..60 | ForEach-Object {
    (Get-Process clash-zhengl).WorkingSet64 / 1MB
    Start-Sleep -Milliseconds 1000
} ; ($all | Sort-Object)[[int](60*0.95)-1]

# 空闲 CPU（10 秒窗口，单核占比）
$p = Get-Process clash-zhengl; $c0 = $p.TotalProcessorTime
$sw = [Diagnostics.Stopwatch]::StartNew(); Start-Sleep -Seconds 10; $sw.Stop()
$p.Refresh()
[math]::Round((($p.TotalProcessorTime - $c0).TotalSeconds / $sw.Elapsed.TotalSeconds) * 100, 2)

# 线程/句柄数（异常增长说明泄漏）
(Get-Process clash-zhengl) | Select-Object Handles, Threads.Count

# 整套目录占用
'{0:N2} MB' -f ((Get-ChildItem $env:LOCALAPPDATA\ClashZhengl -Recurse -File |
    Measure-Object Length -Sum).Sum / 1MB)
```

### 10.3.4 WebView2 内存归因

`clash-zhengl.exe` 只是宿主，WebView2 内存记在 `msedgewebview2.exe` 子进程上（多进程：browser / GPU / renderer / utility）。
**M1/M2 的口径**：`perf-probe.ps1` 把「主进程 RSS」与「WebView2 子树 RSS」分开报。
验收判定用**主进程预算**（90/140 MB）+ **WebView2 ≤ 80 MB**（隐藏时 ≤ 40 MB）作为补充口径。
若两者之和明显超预算，处置顺序见 10.7。

```powershell
# 手工核对 WebView2 子树
$mine = (Get-Process clash-zhengl).Id
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
    Where-Object { $mine -contains $_.ParentProcessId } |
    ForEach-Object { Get-Process -Id $_.ProcessId } |
    Select-Object Id, @{n='RSS_MB';e={[math]::Round($_.WorkingSet64/1MB,1)}}
```

### 10.3.5 后台唤醒测量（M9）

```powershell
# 简易版：对比两个 60s 窗口的 CPU 时间增量，若增量 ≈ 0 说明无唤醒
function Measure-IdleCpu([int]$Sec) {
    $p = Get-Process clash-zhengl; $c0 = $p.TotalProcessorTime
    Start-Sleep -Seconds $Sec; $p.Refresh()
    [math]::Round(($p.TotalProcessorTime - $c0).TotalMilliseconds, 1)
}
$wakeMs = Measure-IdleCpu 60
Write-Host "60s 空闲 CPU 时间增量 = $wakeMs ms"
# 期望 < 900 ms（≈1.5% × 60s）；若 > 1500ms 说明有高频定时器
```

**代码侧自证**：调度器实现「单一 tick」（`scheduler/runner.rs`），tick 间隔必须是 **≥ 30s** 或
对齐到整分；`traffic:tick` 在窗口隐藏时降到 10s 或暂停（C.7）。用 `tracing` 在 tick 处打
`DEBUG [scheduler] tick tasks=N`，跑 10 分钟数日志条数即可验证唤醒频率。

```powershell
# 数 10 分钟内 tick 次数（Debug 级别开启时）
$log = Get-ChildItem "$env:LOCALAPPDATA\ClashZhengl\logs\app-*.log" |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
(Select-String -Path $log.FullName -Pattern '\[scheduler\] tick' -AllMatches).Count
# 期望 ≈ 600s / tick间隔；若 tick=30s → 20 次左右；超过 100 次说明设计有问题
```

---

## 10.4 逐条优化手段（按收益/风险排序）

### 10.4.1 构建期：`Cargo.toml` release profile（收益最大，风险最低）

```toml
# src-tauri/Cargo.toml（必须是 src-tauri 的 Cargo.toml，不是 workspace 根）
[profile.release]
opt-level   = "z"     # 体积优先；对代理类 IO 密集程序性能影响可忽略
lto         = "fat"   # 跨 crate 内联，体积与启动双收益
codegen-units = 1     # 与 fat LTO 配合，进一步压缩
strip       = true    # 去掉符号表（Windows 上 strip 掉 .pdb 引用与调试节）
panic       = "abort" # 见下方取舍
incremental = false
```

| 项 | 收益量级 | 风险 / 取舍 |
| --- | --- | --- |
| `opt-level="z"` | 包体 −15~25% | 数值计算略慢；本项目无重计算，可接受 |
| `lto="fat"` | 包体 −10~20%，启动 −5~15% | 编译时间 ×2~3；CI 可接受 |
| `codegen-units=1` | 包体 −5~10% | 编译更慢 |
| `strip=true` | 二进制 −30~50% | 崩溃栈无符号，靠 `logs/` 定位 |
| `panic="abort"` | 二进制 −5~10% | **代价**：无法 `catch_unwind`，panic 即进程退出。本项目要求「失败不崩」（01 §3.4），因此**只在 panic 全部被消除后**才开启；P6 之前保持默认 `unwind` |

> `panic="abort"` 的启用条件（三条同时满足才允许改）：
> ① `cargo clippy` 全绿且生产路径无 `unwrap/expect`（C.13）；
> ② 所有后台任务用 `tokio::spawn` + 结果收敛，不依赖 `catch_unwind`；
> ③ 托盘与退出路径有「进程异常退出 → 自动重启」的看护（`app.rs` 生命周期）。
> 否则保守留 `unwind`，用体积换稳定性。

**额外体积手段**：

```toml
[dependencies]
# 关闭 reqwest 默认特性里的不需要项
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
# serde_yaml → 「只解析不重排」；不要引入 yaml-rust 全家桶
# 禁止引入：clap（命令行用自定义解析或直接不用）、anyhow+thiserror 并存（选 thiserror）
```

```powershell
# 核对体积构成（Windows 上最快的办法）
Get-Item .\src-tauri\target\release\clash-zhengl.exe | Select-Object Name, @{n='MB';e={[math]::Round($_.Length/1MB,2)}}
# 查看最大的 10 个依赖（需 cargo-bloat，可选）
cargo install cargo-bloat --locked
cargo bloat --release --crates -n 10
```

### 10.4.2 运行时：tokio `current_thread`（收益：内存 −3~8 MB，CPU 抖动下降）

```rust
// main.rs / app.rs
let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .max_blocking_threads(4)      // 默认 512，必须收紧
    .thread_keep_alive(std::time::Duration::from_secs(10))
    .build()?;
```

| 项 | 说明 |
| --- | --- |
| `current_thread` | 单线程事件循环，避免 N 个 worker 线程各带栈（每线程默认 2 MB 保留） |
| `max_blocking_threads(4)` | 文件 IO / DPAPI 调用走 blocking 池；不限会有几十个线程被拉起 |
| **禁止** `#[tokio::main(flavor = "multi_thread")]` | 默认多线程在本项目纯属浪费 |
| 注意 | `current_thread` 下**禁止**在任何 async 路径里同步阻塞（读大文件、`std::fs`、`wait()`），必须 `spawn_blocking` |

### 10.4.3 事件降采样（收益：UI CPU 从 3~5% 降到 0.5% 以下）

| 事件 | C.7 约定 | 实现要点 |
| --- | --- | --- |
| `traffic:tick` | 运行期每 2s；窗口不可见降到 10s 或暂停 | 可见性来自 Rust 侧 `WindowEvent`（**不是 IPC**）；不可见时只更新 `RuntimeState.traffic` 不发事件，恢复可见时用 `core_status` 拉一次 |
| `speedtest:progress` | 每完成 1 个探测，**批量合并 ≤4 条/秒** | 用 `tokio::time::interval(250ms)` 合并发；不要每次探测完成就 emit；负载 > 8 KB 视为设计错误（C.7 v1.1） |
| `core:install-progress` | 下载/校验/解压进度，**≤2 条/秒** | 用 `interval(500ms)` 合并；`phase` ∈ `download\|verify\|extract\|done`；`percent` 单调不减 |
| `core:state` | 状态变化才发 | 加 `if new == old { return }` 去重 |
| `sysproxy:state` | 系统代理状态变化（含被外力改动后探测） | 注册表变更探测用「退出时对比 + 前台 5s 轮询」；**只在值真变时发** |
| `subscription:updated` | 单订阅刷新成功/失败 | 天然低频，不用限频 |
| `notify` | 需要浮出时 | 前端做 token bucket（10s 内最多 3 条） |

> **总原则**（C.7 末段）：任何事件频率 > 5 Hz 必须在后端合并；单条负载 > 8 KB 视为设计错误。
> `speedtest:progress` 尤其**不得携带全量 results**——只带 `latest` 一条。

```rust
// speedtest 进度合并示例（伪代码）
let (tx, mut rx) = tokio::sync::mpsc::channel::<ProbeResult>(256);
// 采集侧：每完成一个就 tx.send(...).await
// 发射侧：
let mut tick = tokio::time::interval(Duration::from_millis(250));
loop {
    tokio::select! {
        _ = tick.tick() => {
            if buf.is_empty() { continue; }
            emit("speedtest:progress", json!({"runId": id, "finished": n, "total": total,
                                              "latest": buf.last().cloned()}));
            buf.clear();
        }
        Some(r) = rx.recv() => { buf.push(r); n += 1; if buf.len() >= 8 { /* 立即发 */ } }
    }
}
```

> 前端侧铁律（与 09 章一致）：收到 `speedtest:progress` **只更新单行 DOM 的 textContent**，
> 禁止重建整个节点列表；节点数 > 300 时用虚拟滚动，只渲染视口内 ~40 行。

### 10.4.4 WebView 隐藏暂停渲染（收益：隐藏态 RSS −20~40 MB，CPU −1~3%）

| 手段 | 做法 | 收益 |
| --- | --- | --- |
| 窗口 `hide()` 而非 `destroy` | 复用实例，避免重建开销 | 启动快 |
| 隐藏时停止 rAF 驱动的动画 | 前端 `document.visibilityState === 'hidden'` 时 `cancelAnimationFrame` | CPU |
| 隐藏时停用 CSS 动画/过渡 | `body.hidden { animation: none !important; transition: none; }` | GPU/CPU |
| 隐藏时清空大列表 DOM | 订阅数多时卸载节点列表，可见时重渲染 | 渲染内存 |
| 禁用不必要的 WebView 特性 | Tauri 2 `webview.on_page_load` + 前端 `concurrency` 简化 | 启动 |
| 关闭 devtools（release） | `tauri.conf.json` 中 `devtools: false` | 内存 |

```ts
// src/main.ts：隐藏时停掉前端侧开销
// 注意：不要为此新增 IPC（C.6 v1.1 明确「不做 ui_visibility_set」）。
// 窗口可见性由 Rust 侧监听 Tauri WindowEvent（Focused/Resized/最小化）自行判断，
// 前端这里只负责「停自己的动画与重渲染」，不向后端发心跳。
document.addEventListener('visibilitychange', () => {
  const hidden = document.visibilityState === 'hidden';
  document.body.classList.toggle('is-hidden', hidden);
  if (hidden) {
    cancelAnimationFrame(window.__zhengl.rafId);      // 停止 rAF 循环
    window.__zhengl.suspendListRendering();           // 暂停列表虚拟滚动计算
  } else {
    window.__zhengl.resumeAndRefreshOnce();           // 恢复后主动刷新一次状态
  }
});
```

```rust
// 后端：可见性由 Tauri 窗口事件驱动，不新增 IPC 命令（C.6 v1.1）
// src-tauri/src/app.rs
window.on_window_event(move |event| {
    let visible = matches!(event, tauri::WindowEvent::Focused(true))
        || matches!(event, tauri::WindowEvent::Resized(_));
    // 写入共享状态，traffic 推送任务读取它决定 2s / 10s / 暂停
    state.ui_visible.store(visible, std::sync::atomic::Ordering::Relaxed);
});
```

```rust
// traffic 推送任务：按可见性调整频率（C.7 要求「窗口不可见时降到 10s 或暂停」）
let interval = if ui_visible.load(Relaxed) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(10)      // 或直接暂停，只在恢复可见时拉一次 core_status
};
```

> **不要**为了省内存而销毁 WebView 再重建：重建会导致明显的白屏与 1s+ 延迟，
> 反而破坏「1.2s 托盘可用」的体验。隐藏即可。
>
> **也不要**把 `ui_visibility_set` 加进 IPC 表：C.6 v1.1 已裁决「窗口可见性由 Rust 侧自行判断，
> 不新增 IPC，前端不得为降采样再发心跳」。加了就是契约违规（12.4 契约测试会拦下）。

### 10.4.5 YAML 不常驻（收益：RSS −10~40 MB，视订阅规模）

| 反模式 | 问题 | 正确做法 |
| --- | --- | --- |
| `RuntimeState` 里存 `Vec<ProxyNode>` 完整结构 | 500 节点 × 深拷贝 ≈ 10~30 MB | 只在**合并时**短暂持有；运行态只留 `node_key → Option<delay>` |
| 每次刷新都 `String` 全量 clone 三份 | 堆碎片与峰值翻倍 | 用 `&str` 切片 + `serde_yaml::Value` 就地消费；合并后立刻 `drop` |
| `serde_yaml::Value` 常驻缓存 | `Value` 树比原文大 3~6 倍 | 只在 `subscription/parse.rs` 内部存在，函数返回即释放 |
| 把 `runtime/config.yaml` 全文读到内存做字符串替换 | 8 MB 配置 × 多次 = 峰值 30 MB+ | 流式写：`BufWriter` 逐段 `write!`，不要 `format!` 拼大串 |

```rust
// 正确：流式生成 runtime/config.yaml（峰值内存 ≈ 单个节点的序列化大小）
use std::io::Write;
let f = std::fs::File::create(&tmp_path)?;
let mut w = std::io::BufWriter::with_capacity(64 * 1024, f);
writeln!(w, "mixed-port: {}", port)?;
writeln!(w, "external-controller: 127.0.0.1:{}", api_port)?;
writeln!(w, "secret: \"{}\"", secret)?;
write!(w, "proxies: [")?;
for (i, p) in merged.iter().enumerate() {
    if i > 0 { write!(w, ",")?; }
    write!(w, "\n  {}", serde_json::to_string(p)?)?;   // YAML 是 JSON 超集
}
writeln!(w, "\n]")?;
w.flush()?;
drop(w);                       // 关键：立刻释放缓冲
atomic_rename(&tmp_path, &final_path)?;
```

> **YAML 是 JSON 超集**：节点对象用 `serde_json::to_string` 直接写进 YAML 是合法的，
> 比引入 YAML 序列化器更快、峰值更低。解析仍需 `serde_yaml`。

**上限保护**（同时防内存与防恶意订阅）：

| 限制 | 值 | 行为 |
| --- | --- | --- |
| 单订阅响应体 | 8 MB | 超过 → `E_PARSE_YAML`（或先 `Content-Length` 预检） |
| 合并后节点数 | 5000 | 超过 → 截断并 Warn，UI 提示 |
| 合并后配置体积 | 32 MB | 超过 → 拒绝写入，`E_STORE_IO` |
| `proxies` 单条字段数 | 64 | 超过 → 跳过该节点并 Warn（防畸形订阅炸解析器） |

### 10.4.6 日志缓冲写与滚动（收益：CPU −0.3~1%，IO 抖动消失）

```rust
// util/log.rs
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::writer::MakeWriterExt;

let file = RollingFileAppender::builder()
    .rotation(Rotation::DAILY)
    .filename_prefix("app")
    .max_log_files(3)                 // 保留 3 天（C.3）
    .build(&log_dir)?;

let (nb, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
    .lossy(true)                      // 关键：满了丢日志，绝不阻塞业务线程
    .buffered_lines_limit(4096)       // 缓冲行数上限
    .finish(file);

tracing_subscriber::registry()
    .with(fmt::layer().with_writer(nb).with_ansi(false))
    .with(RedactLayer)                // 脱敏层（C.9）
    .init();
// guard 必须在退出时 drop/flush，否则丢尾部日志
```

| 项 | 说明 |
| --- | --- |
| `lossy(true)` | 磁盘慢时宁可丢日志也不能卡住测速线程 |
| 缓冲 | 默认 128 KB 块写；不要 `File::open` 后每行 `write_all` + `flush` |
| 单文件上限 | C.3 要求 2 MB × 3 天；`RotatingFileAppender` 按天切，需另加大小守卫：写入前检查 `metadata().len()`，超 2 MB 触发 `rollover` 或直接换新文件 |
| 级别默认 | `Info`；`Debug` 只在用户开启时启用，且 Debug 下必须限流（每类请求 ≤ 10 条/秒） |
| 内核 stdout | 单独文件 `core-YYYYMMDD.log`（C.3），同样 non-blocking + 过滤 `secret` |

```rust
// 大小守卫（C.3 的 2 MB 硬要求）
fn rotate_if_too_large(path: &Path, limit: u64) -> std::io::Result<()> {
    if let Ok(m) = std::fs::metadata(path) {
        if m.len() > limit {
            let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
            let bak = path.with_file_name(format!("app-overflow-{ts}.log"));
            std::fs::rename(path, bak)?;
            prune_old_logs(path.parent().unwrap(), 3)?;   // 只留 3 个文件
        }
    }
    Ok(())
}
```

### 10.4.7 图标与资源裁剪（收益：包体 −1~3 MB）

| 资源 | 现状风险 | 要求 |
| --- | --- | --- |
| 应用图标 | 一套 1 MB+ 的多尺寸 ico | 只保留 16/24/32/48/64/128/256 七个尺寸，PNG 压缩后总量 ≤ 200 KB |
| 托盘图标 | 用 256×256 缩放 | 托盘单独用 16/32 px 两档，内嵌 `include_bytes!` |
| 安装器横幅 | 248×58 BMP 未压缩 | 用 24 位 BMP 精简版本，≤ 50 KB |
| 安装器侧栏 | 164×314 | 同上 |
| 前端字体 | 引入完整 CJK 字体 4~8 MB | **禁止**打包中文字体；用系统 `Microsoft YaHei UI` 字体栈 |
| 前端图片 | 空态插画用 PNG | 用 SVG 内联（每个 ≤ 3 KB） |
| i18n | 只留 `zh-CN.json` | `en-US` 预留键但**不打包**内容（或用极小占位），避免 2 份文案 |
| 前端依赖 | 引入 lodash/moment 等 | 禁止；日期格式化用 `Intl.DateTimeFormat` |

```powershell
# 核对前端产物里最大的 10 个文件
Get-ChildItem .\dist -Recurse -File |
    Sort-Object Length -Descending | Select-Object -First 10 Name, @{n='KB';e={[math]::Round($_.Length/1KB,1)}}

# 核对图标体积
Get-ChildItem .\src-tauri\icons -File | Select-Object Name, @{n='KB';e={[math]::Round($_.Length/1KB,1)}}
```

```jsonc
// tauri.conf.json —— 只留必要的 bundle 资源
{
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "icon": ["icons/32x32.png", "icons/128x128.png", "icons/icon.ico"],
    "resources": [],                        // 不要把 docs/ 或 examples 打进去
    "windows": { "nsis": { "compression": "lzma", "installMode": "currentUser" } }
  }
}
```

### 10.4.8 NSIS 压缩与安装包瘦身（收益：包体 −1~4 MB）

| 手段 | 做法 | 说明 |
| --- | --- | --- |
| `compression: "lzma"` | Tauri NSIS 配置项 | 比默认 zlib 小 25~40% |
| `installMode: "currentUser"` | 免 UAC、免管理员 | 与「默认非管理员运行」（C.12.4）一致 |
| 不捆绑内核 | 首启下载（C.12.5 策略 a） | 省 10~20 MB，且规避 GPL 传染 |
| 不捆绑 WebView2 离线安装器 | 用 bootstrapper（约 2 MB）或检测已装 | 离线安装器约 130 MB，绝对不打包 |
| 语言包 | NSIS `languages: ["SimpChinese"]` | 每个语言约 5~20 KB，但只留一个更干净 |
| `strip` 后的 exe | 见 10.4.1 | 直接决定主二进制大小 |

```jsonc
// tauri.conf.json → bundle.windows.nsis
{
  "nsis": {
    "compression": "lzma",
    "installMode": "currentUser",
    "languages": ["SimpChinese"],
    "displayLanguageSelector": false
  }
}
```

### 10.4.9 按需启动内核（收益：空闲 RSS −15~30 MB，CPU −0.2%）

| 场景 | 策略 |
| --- | --- |
| 用户只用「看流量/到期」，不开代理 | 设置里 `start_core_on_launch` 可关；关时**完全不 spawn 内核** |
| 系统代理未开启且模式为 `Manual` | 内核可停；UI 明确显示「内核未运行」 |
| 切节点/测速前 | 必须先确保内核 Running，否则返回 `E_CORE_NOT_FOUND` |
| 自动测速定时器 | 内核未运行时**跳过本轮**并记 Info，不为了测速去拉起内核（除非用户在设置里勾选「测速时按需启动内核」） |
| 退出 | 先关系统代理 → 再停内核 → 再退进程，顺序不可颠倒 |

```rust
// 关键：内核就绪才允许测速/切换
if !matches!(state.core, CoreState::Running) {
    return Err(AppError::new(ErrorCode::CoreNotFound, "未安装内核，请先下载"));
}
```

> 这一条是**最大的单笔内存收益**：mihomo 自身常驻约 25~60 MB（视规则数量）。
> 但它不是「我们可以随便超标」的理由——M1 的 90 MB 预算里已经**包含内核**吗？
> **不包含**。C.11 的 M1 指「应用进程 RSS」，即 `clash-zhengl.exe`。内核内存单独记账，
> 并要在文档里如实报告（10.6 的基线表有独立一列）。

### 10.4.10 定时器合并与对齐（收益：CPU −0.3~1%，M9 达标的关键）

| 规则 | 说明 |
| --- | --- |
| 单一 tick 线程 | `scheduler/runner.rs` 一个 tick，间隔 30s 或 60s，所有周期任务由它派生 |
| 对齐整分 | tick 对齐到 `:00` 秒触发，避免用户设备上多个定时器错峰唤醒 CPU |
| 启动抖动 | 所有周期任务首次触发加 `0..30s` 随机偏移（C.10） |
| 合并同刻任务 | 同一 tick 内到期的订阅更新按 `sub_fetch_concurrency`（默认 2）串行，不并发风暴 |
| 睡眠优先 | 无可执行任务时 `tokio::time::sleep` 到底，禁止 `loop + sleep(1s)` 轮询 |
| 时间跳变 | 用单调时钟算间隔，`SystemTime` 只用于计算到期；睡眠唤醒后重新计算 next_due（详见 05 章） |

```rust
// runner.rs 骨架（对齐整分 + 抖动）
async fn run(mut self) {
    let jitter = rand::random::<u64>() % 30;
    tokio::time::sleep(Duration::from_secs(jitter)).await;
    loop {
        self.tick_once().await;                       // 纯逻辑：算出到期集合，串行执行
        let now = chrono::Local::now();
        let next = now + chrono::Duration::seconds(60)
            - chrono::Duration::seconds(now.timestamp() % 60);   // 对齐到整分
        let wait = (next - chrono::Local::now()).to_std().unwrap_or(Duration::from_secs(60));
        tokio::time::sleep(wait).await;
    }
}
```

### 10.4.11 依赖白名单重新评审（每次新增依赖前必做）

| 依赖 | 体积影响 | 结论 |
| --- | --- | --- |
| `reqwest`（rustls） | +1.5~2.5 MB | 必需；关默认特性、只留 `rustls-tls` + `json` |
| `serde_yaml` / `serde_yaml_ng` | +300~500 KB | 必需 |
| `tracing` + `tracing-subscriber` + `tracing-appender` | +400~700 KB | 必需 |
| `chrono` | +200~400 KB | 可用 `time` 替代，但 chrono 更常用；允许 |
| `windows`（Win32 API） | 视 feature 而定，+200 KB~1 MB | 只开用到的 feature（`Win32_Networking_WinInet`、`Win32_Security_Cryptography`） |
| `rand` | +30 KB | 必需（secret、抖动） |
| `zip` | +200~400 KB | 内核解压必需；只用 `deflate` feature |
| `tokio` | +400~800 KB | 必需；`features = ["rt", "time", "sync", "macros", "fs", "net"]`，不要 `full` |
| ❌ `clap` | +300 KB | 本项目无 CLI，禁止 |
| ❌ `anyhow`（与 thiserror 并存） | +50 KB | 只选 `thiserror`，自定义 `AppError` |
| ❌ `tauri-plugin-*` 全家桶 | 每个 +200 KB~2 MB | 按需，且必须论证 |
| ❌ `sqlx` / `rusqlite` | +1~3 MB | 单文件 JSON 足够（C.3） |
| ❌ `image` / `font-kit` | +1 MB+ | 图标构建期处理，不进运行时 |

```powershell
# 依赖树体积核查（需 cargo-bloat）
cargo bloat --release --crates -n 20
# 依赖数量与重复版本核查
cargo tree --duplicate
```

---

## 10.5 分阶段性能检查点（不要把性能留到最后）

| 阶段 | 必测指标 | 通过线 | 不通过时 |
| --- | --- | --- | --- |
| P0 脚手架 | M5（空窗口启动） | ≤ 800 ms（给 M6 留 400 ms 余量） | 检查 WebView2 初始化、是否加载了远程资源 |
| P1 内核与系统代理 | M1 + M6 | RSS ≤ 70 MB、代理可用 ≤ 3s | 查内核参数、是否 `-d` 指向了错误目录导致重建缓存；`runtime/download/` 下载完成后必须清理 |
| P2 订阅合并 | 峰值内存（合并 500 节点时） | 峰值增量 ≤ 50 MB | 检查 YAML 是否常驻、是否多次 clone |
| P3 调度器 | M3 + M9 | CPU ≤ 1.0%、tick ≥ 30s | 查是否有 `sleep(1s)` 轮询 |
| P4 测速与切换 | M4 + 事件频率 | CPU ≤ 25%、事件 ≤ 4/s | 信号量是否生效、进度是否合并 |
| P5 UI 与托盘 | M1 + M2 | 90 / 140 MB | 虚拟滚动、隐藏暂停渲染 |
| P6 轻量化调优 | 全指标 | 全部达标 | 按 10.7 阶梯 |
| P7 打包 | M7 + M8 | 12 MB / 120 MB（C.11 v1.1 口径） | lzma、图标裁剪、不捆内核 |
| P8 验收 | 全指标 × 3 次 | 中位数达标 | 回 P6 |

> **100%–120% 区间怎么处理**（C.11 v1.1）：在该阶段允许带着超支进入下一阶段，
> 但必须在 `docs/perf-log.md` 记一条「指标 / 实测 / 超支百分比 / 下一阶段改进项」，
> 并在下一阶段结束时复查。**超过 120% 不允许带病通过**，必须当场按 10.7 处置。

---

## 10.6 基线记录模板（贴进 `docs/perf-baseline.md`）

```markdown
## 基线 v0.1.0-rc1（2026-11-06）

环境：Win11 23H2 x64 / 4C8G / 无独立显卡 / 订阅 3 条（共 386 节点）

| 指标 | 预算 | 实测 | 结论 |
| --- | --- | --- | --- |
| M1 主进程 RSS（隐藏+内核） | ≤ 90 MB | 64.2 MB (P95) | PASS |
| M1' WebView2 子树（隐藏） | ≤ 40 MB | 31.5 MB | PASS |
| M2 主进程 RSS（可见） | ≤ 140 MB | 88.7 MB (P95) | PASS |
| M3 空闲 CPU | ≤ 1.5% | 0.42% | PASS |
| M4 测速期 CPU | ≤ 25% / ≤5s | 18.3% / 3.1s | PASS |
| M5 冷启动到托盘 | ≤ 1.2 s | 741 ms (中位数 5 次) | PASS |
| M6 含内核代理可用 | ≤ 3 s | 1.86 s | PASS |
| M7 安装包体积 | ≤ 12 MB | 8.4 MB | PASS |
| M8 常驻磁盘 | ≤ 120 MB | 46.8 MB | PASS |
| M9 后台唤醒 | 无 <30s 唤醒 | tick=60s，10min 内 11 次 | PASS |
| 参考：mihomo 内核 RSS | 不入预算 | 38.1 MB | 记录 |

命令：`pwsh -File .\scripts\perf-probe.ps1 -Mode all -Minutes 5 -OutJson .\docs\perf\rc1.json`
```

---

## 10.7 不达标降级阶梯（**必须按顺序**执行，不许跳级）

每降一级都要重测并把数字记进基线表；降级必须在 PR 里写明「降了哪一级、为什么、影响什么」。

### L0 · 先排除测量误差（0 成本）

| 检查 | 做法 |
| --- | --- |
| 确认是 release | `Get-Item clash-zhengl.exe` 的 `Length` 应 < 15 MB；debug 版通常 30 MB+ |
| 确认无 devtools | `tauri.conf.json` 中 `devtools: false`；调试器附加会使 RSS +20 MB |
| 确认无其它实例残留 | `Get-Process clash-zhengl` 只有一个 |
| 采样是否预热 | 启动后 30s 再采；未预热时 RSS 偏低、CPU 偏高 |
| 是否被 Defender 扫描 | 首次运行有一次性 CPU 峰值，第二次测 |
| 对比基线 | 与 `docs/perf-baseline.md` 上一次数字比；若基线本来就超，先修基线 |

### L1 · 降采样频率（仅影响实时性，不影响正确性）

| 参数 | 从 | 到 | 影响 |
| --- | --- | --- | --- |
| `traffic:tick` 可见时 | 2 s | 3 s | 流量数字刷新慢 1s，用户基本无感 |
| `traffic:tick` 隐藏时 | 10 s | 30 s | 无感 |
| `speedtest:progress` 合并窗口 | 250 ms | 500 ms（即 ≤2 条/秒） | 进度条略顿 |
| 空闲 tick | 60 s | 120 s | 定时精度变差（仍在 C.10 ±10% 内需留意） |

> 触发条件：M3 超 1.5% 但 < 3%，且日志显示 tick 次数明显高于设计。

### L2 · 关内核按需启动（影响功能可用性，需 UI 明示）

| 动作 | 说明 |
| --- | --- |
| 默认 `start_core_on_launch = false` | 首启进托盘不拉内核；用户开「系统代理」或点「启动内核」才拉 |
| 自动测速前不自动拉内核 | 内核未运行 → 跳过本轮，UI 显示「内核未运行，已跳过自动测速」 |
| 空闲 30 分钟且系统代理关闭 → 自动停内核 | 需在设置里可关（默认关），并在 UI 提示 |

> 触发条件：M1 超 90 MB 且已排除 WebView2 因素，且用户场景以「看流量」为主。

### L3 · 换 UI 渲染策略（影响观感，不影响功能）

| 手段 | 收益 | 代价 |
| --- | --- | --- |
| 节点列表改分页（每页 50）替代全量渲染 | 渲染内存 −20~50 MB | 交互变差 |
| 关闭所有 CSS 动画与毛玻璃/阴影 | CPU −0.5~1% | 观感下降 |
| 图表/进度条改为纯文本 | 渲染层简化 | 观感下降 |
| 隐藏时**卸载整个页面 DOM**（保留空壳） | 隐藏态 −10~25 MB | 再次显示有 100~300 ms 重绘 |

> 触发条件：M2 超 140 MB。**优先检查是否引入了框架**——原生 DOM 方案不应到 140 MB。

### L4 · 结构性降级（需用户/Lead 决策，走 CONTRACT-CHANGE-REQUEST）

| 手段 | 说明 |
| --- | --- |
| 去掉 WebView，改纯原生 Win32 托盘 + 设置窗 | 内存可降到 20~40 MB，但等于重写 UI 层，与 02 章选型冲突 → 必须 Lead 决策 |
| 内核长驻改为「仅系统代理开启时运行」 | 已含在 L2，L4 指**强制**策略，不可关闭 |
| 降低功能面（不做实时流量曲线，只做数字） | 产品能力裁剪，需用户同意 |
| 接受 M1 放宽到 110 MB | 改契约 C.11，必须 `CONTRACT-CHANGE-REQUEST:` |

> **L4 的前置动作**：先写一份 `docs/perf-exception.md`，包含：①实测数据 ②已尝试的 L0–L3 ③
> 为什么结构性方案是唯一出路 ④对产品能力的影响 ⑤建议的契约修订。**没有这份文档不许改预算。**

### 10.7.1 降级决策速查

```text
M5 启动超 1.2s ──────────→ L0（先确认 release / 无 devtools）→ 检查是否加载远程资源 → L3（减少隐藏 DOM）
M1 空闲 RSS 超 90MB ─────→ L0 → L1（tick）→ 检查 WebView2 子树 → L2（按需内核）→ L3 → L4
M2 可见 RSS 超 140MB ────→ 检查是否引入框架 → L3（虚拟滚动/分页）→ L4
M3 空闲 CPU 超 1.5% ─────→ 查 tick 频率与日志量 → L1 → 查是否有 1s 轮询 → L2
M4 测速 CPU 超 25% ──────→ 降 speedtest_concurrency（8→4）→ 提高 timeout → 检查事件合并
M7 包体超 12MB ──────────→ lzma + strip + 不捆内核 + 图标裁剪 + 依赖白名单复核
M8 磁盘超 120MB ─────────→ 旧内核目录清理 + 日志保留 3 天 + 旧 profile 清理
M9 存在 <30s 唤醒 ───────→ 合并定时器 / tick 对齐整分 / 去掉 1s 轮询
```

---

## 10.8 性能反模式黑名单（代码评审直接打回）

| # | 反模式 | 后果 | 替代 |
| --- | --- | --- | --- |
| 1 | `#[tokio::main]`（默认多线程） | 多 worker 线程，RSS +5~10 MB | `current_thread` |
| 2 | `loop { sleep(1s); check(); }` | 每秒唤醒，M9 必挂 | 单一 tick 对齐整分 |
| 3 | 每次探测完成就 `emit` | 500 节点 = 500 次事件风暴 | 250ms 合并 |
| 4 | 前端收到事件就 `innerHTML = ...` 重建列表 | 主线程卡顿、CPU 尖峰 | 只改 `textContent`；虚拟滚动 |
| 5 | 把 `runtime/config.yaml` 全文读进 `String` 做替换 | 峰值内存翻倍 | `BufWriter` 流式写 |
| 6 | `serde_yaml::Value` 塞进 `AppState` | RSS +10~40 MB | 只在解析函数内存在 |
| 7 | 每行日志 `flush()` | IO 抖动、CPU +0.5% | non-blocking + `lossy(true)` |
| 8 | `include_str!` 打包大 JSON（geo 数据） | 包体 +10~50 MB | 运行时按需下载或不用 |
| 9 | 打包中文字体 | 包体 +4~8 MB | 系统字体栈 |
| 10 | 引入 React/Vue/antd | 包体 +150~400 KB、RSS +15~30 MB | 原生 DOM |
| 11 | 用 `std::fs` 在 async 上下文读大文件 | 阻塞事件循环，UI 卡死 | `spawn_blocking` / `tokio::fs` |
| 12 | 每个网络请求新建 `reqwest::Client` | 连接池失效、CPU +1% | 全局单例 `Client` |
| 13 | 定期轮询 `/connections` 或 `/rules` | 响应体巨大，CPU/内存尖峰 | 仅诊断导出时取一次 |
| 14 | 保留全部历史 `ProbeResult` | 内存线性增长 | 只留最近 1 轮 + `SwitchRecord` 上限 200 条 |
| 15 | 日志 `Debug` 级别常开且打印请求体 | 磁盘暴涨、CPU +1% | 默认 `Info`，Debug 需用户开启且限流 |

---

## 10.9 性能相关的自动化守卫（放进 CI）

```powershell
# scripts/build.ps1 尾部追加：构建后体积红线（超了直接失败）
$exe = "src-tauri\target\release\clash-zhengl.exe"
$sizeMB = (Get-Item $exe).Length / 1MB
Write-Host ("exe = {0:N2} MB" -f $sizeMB)
if ($sizeMB -gt 15) { throw "主二进制超过 15 MB（release+strip 后应 <15 MB），请检查依赖与 profile" }

$setup = Get-ChildItem src-tauri\target\release\bundle\nsis -Filter '*_x64-setup.exe' -ErrorAction SilentlyContinue
if ($setup) {
    $pkgMB = $setup.Length / 1MB
    Write-Host ("installer = {0:N2} MB" -f $pkgMB)
    if ($pkgMB -gt 12) { throw "安装包超过 12 MB（C.11 M7），请检查 lzma / 图标 / 是否误捆内核" }
}
```

```yaml
# .github/workflows/ci.yml 片段（windows-latest）
- name: Size guard
  shell: pwsh
  run: |
    $exe = Get-Item src-tauri/target/release/clash-zhengl.exe
    $mb = [math]::Round($exe.Length/1MB,2)
    Write-Host "exe=$mb MB"
    if ($mb -gt 15) { exit 1 }
```

> CI 里**不要**跑 RSS/CPU 门禁：GitHub Runner 的环境噪声大，数字不可比。
> 性能门禁只在本地/专用机跑，CI 只守「包体」这一项静态指标。

---

## 10.10 交付自检清单（P6 / P8 用）

- [ ] `Cargo.toml` release profile 已按 10.4.1 配置，且 `panic="abort"` 有开/关的明确结论与理由
- [ ] `tokio` 为 `current_thread`，`max_blocking_threads` 已收紧，无 `std::fs` 阻塞 async
- [ ] `traffic:tick` / `speedtest:progress` 已降采样，实测事件频率 ≤ 4 条/秒
- [ ] 窗口隐藏时 traffic 推送降频或暂停，实测隐藏态 RSS 明显低于可见态
- [ ] `runtime/config.yaml` 流式生成，实测合并 500 节点峰值增量 ≤ 50 MB
- [ ] 日志 non-blocking + `lossy(true)` + 单文件 2 MB 守卫 + 保留 3 天
- [ ] 图标资源按 10.4.7 裁剪，前端无中文字体、无框架、无大图
- [ ] NSIS 已开 `lzma` + `currentUser`，未捆内核、未捆 WebView2 离线包
- [ ] `perf-probe.ps1` 全模式可跑，`-Mode all` 退出码 0
- [ ] `docs/perf-baseline.md` 已按 10.6 模板填写，包含内核内存参考值
- [ ] 若有任何指标未达标：已按 10.7 阶梯处置并留下 `docs/perf-exception.md`
- [ ] README/About 页的「资源占用」宣传语与实际实测数字一致（不得夸大）

---

## 契约对齐记录（v1.3）

| 契约条目 | 本章落点 | 状态 |
| --- | --- | --- |
| C.11 · 九项预算 M1–M9 | 10.1 预算表（数字与 C.11 完全一致，未改写） | ✅ 已落地 |
| C.11 · M7 口径 = **签名后** `setup.exe`，不含内核/WebView2 | 10.1 M7 行 + 10.3.1 `Invoke-DiskProbe` | ✅ 已落地 |
| C.11 · M8 口径 = 安装目录 + `%LOCALAPPDATA%`，**不含** `%APPDATA%` 订阅原文 | 10.1 M8 行 + 10.3.1（`%APPDATA%` 仅打印参考值、不计分） | ✅ 已落地 |
| C.11 v1.1 · **100%–120% 过渡仲裁**（可保留但须记 `perf-log.md` 且下阶段改进；>120% 不达标） | 10.1 仲裁说明 + 10.5 检查点表下方 | ✅ 已落地 |
| C.6 v1.1 · **明确不做 `ui_visibility_set`** | 10.4.4 前端示例改为「不发心跳」+ Rust `WindowEvent` 驱动 + 反面说明 | ✅ 已落地 |
| C.7 v1.1 · 事件限流（`speedtest:progress` ≤4 条/秒、`core:install-progress` ≤2 条/秒、单条 ≤8 KB） | 10.4.3 事件降采样表（含新增两行 + 总原则） | ✅ 已落地 |
| C.3 v1.1 · `runtime/download/` 续传区 | 10.5 P1 检查点（下载完成后必须清理，否则 M8 超预算） | ✅ 已落地 |

**未决项**：无。本章数字全部取自 C.11，不涉及 v1.2 / v1.3 的新增类型。

> **提醒（口径与 C.11 的已知张力，非遗留问题）**：C.11 的 90 MB 指 `clash-zhengl.exe` 主进程，
> **不含 mihomo 内核**（内核另计 25–60 MB）。本章 10.1 的预算分解表已按「主进程 + WebView2 子树」拆开列，
> 测量时须用 `perf-probe.ps1` 分别记录主进程与内核进程，不要合并成单个数字报出。
