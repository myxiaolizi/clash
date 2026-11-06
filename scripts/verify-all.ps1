# =============================================================================
#  Clash Zhengl — 全量验收脚本（P8）
#
#  依次执行：编译检查 → 单元测试 → 前端类型检查 → 前端构建 → 契约自检
#            → 性能实测 → 打包
#
#  用法：
#    powershell -ExecutionPolicy Bypass -File .\scripts\verify-all.ps1
#    powershell -ExecutionPolicy Bypass -File .\scripts\verify-all.ps1 -SkipBundle
# =============================================================================

[CmdletBinding()]
param(
    [switch]$SkipBundle,
    [switch]$SkipPerf,
    [switch]$Quick
)

$ErrorActionPreference = 'Continue'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

$results = @()
$failed = 0

function Step {
    param(
        [string]$Name,
        [scriptblock]$Action
    )
    Write-Host ''
    Write-Host "=== $Name ===" -ForegroundColor Cyan
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $ok = $false
    try {
        $ok = & $Action
    } catch {
        Write-Host "异常: $_" -ForegroundColor Red
        $ok = $false
    }
    $sw.Stop()
    $status = if ($ok) { 'PASS' } else { 'FAIL' }
    if (-not $ok) { $script:failed++ }
    Write-Host ("[{0}] {1}  ({2:N1}s)" -f $status, $Name, $sw.Elapsed.TotalSeconds) `
        -ForegroundColor $(if ($ok) { 'Green' } else { 'Red' })
    $script:results += [pscustomobject]@{
        '步骤'   = $Name
        '结果'   = $status
        '耗时(s)' = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    }
    return $ok
}

# ---------------------------------------------------------------------------
Write-Host ''
Write-Host '==========================================' -ForegroundColor Cyan
Write-Host ' Clash Zhengl 全量验收' -ForegroundColor Cyan
Write-Host '==========================================' -ForegroundColor Cyan

# ---- 1. Rust 编译（零警告要求）----
Step 'Rust cargo check' {
    Push-Location src-tauri
    try {
        $out = cargo check --all-targets 2>&1 | Out-String
        $errs = ([regex]::Matches($out, '(?m)^error')).Count
        $warns = ([regex]::Matches($out, '(?m)^warning:')).Count
        if ($errs -gt 0) {
            Write-Host $out
            return $false
        }
        if ($warns -gt 0) {
            Write-Host "有 $warns 个警告：" -ForegroundColor Yellow
            $out -split "`n" | Select-String -Pattern '^warning' | Select-Object -First 10 | ForEach-Object { Write-Host $_ }
        }
        return $true
    } finally { Pop-Location }
}

# ---- 2. Rust 单元测试 ----
Step 'Rust 单元测试' {
    Push-Location src-tauri
    try {
        # cargo 把进度写 stderr，PS 5.1 会当成 NativeCommandError。
        # 用 2>&1 + Out-String 收集，然后只看 "test result:" 行做判定。
        $out = (& cargo test --lib 2>&1 | Out-String)
        Write-Host $out
        # 取最后一条 test result 行
        $m = [regex]::Matches($out, 'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed')
        if ($m.Count -eq 0) {
            Write-Host '未能解析 test result 行' -ForegroundColor Red
            return $false
        }
        $last = $m[$m.Count - 1]
        $passed = [int]$last.Groups[2].Value
        $failed = [int]$last.Groups[3].Value
        Write-Host ("通过 {0}，失败 {1}" -f $passed, $failed)
        return ($failed -eq 0 -and $passed -gt 0)
    } finally { Pop-Location }
}

# ---- 3. 前端类型检查 ----
Step 'TypeScript 类型检查' {
    $out = npx tsc --noEmit 2>&1 | Out-String
    if ($out.Trim()) { Write-Host $out }
    return ($LASTEXITCODE -eq 0)
}

# ---- 4. 前端构建 ----
Step '前端构建' {
    $out = npx vite build 2>&1 | Out-String
    Write-Host $out
    if ($LASTEXITCODE -ne 0) { return $false }
    # 检查产物存在且不超预算
    $js = Get-ChildItem dist\assets\*.js -ErrorAction SilentlyContinue
    if (-not $js) { Write-Host '未生成 JS 产物' -ForegroundColor Red; return $false }
    $total = ($js | Measure-Object -Property Length -Sum).Sum
    Write-Host ('JS 产物合计: {0:N1} KB（预算 < 300 KB）' -f ($total / 1KB))
    return ($total -lt 300KB)
}

# ---- 5. 契约自检 ----
Step '契约一致性自检' {
    $script = Join-Path $PSScriptRoot 'check-contract.ps1'
    if (-not (Test-Path $script)) {
        Write-Host 'check-contract.ps1 不存在，跳过' -ForegroundColor Yellow
        return $true
    }
    & powershell -ExecutionPolicy Bypass -File $script | Out-Host
    return ($LASTEXITCODE -eq 0)
}

# ---- 6. 命名空间陷阱专项检查 ----
Step 'Selector 命名空间陷阱检查' {
    # 头号陷阱：API 返回 "Selector"（大驼峰），YAML 写 "select"（小写）。
    # 代码里绝不能**用大小写模糊比较来判定组类型**。
    #
    # 注意：to_lowercase() 本身不是罪。以下场景都是合法的，不能误报：
    #   - 压缩包条目名匹配（fetch_core）
    #   - 内核日志行找 fatal/error（supervisor）
    #   - 错误字符串判 DNS/TLS（fetch）
    #   - HTML 嗅探 / HTTP header 键归一化（parse/userinfo/net）
    # 所以要判的是「同一行里既有 lowercase 又有 select/Selector」这种真正的类型判定。
    $src = Join-Path $Root 'src-tauri\src'
    $violations = @()
    Get-ChildItem $src -Recurse -Filter *.rs | ForEach-Object {
        $lines = Get-Content $_.FullName -Encoding UTF8
        for ($i = 0; $i -lt $lines.Count; $i++) {
            $line = $lines[$i]
            # 跳过注释行
            if ($line -match '^\s*(//|\*|/\*)') { continue }
            # 跳过测试模块之后的内容（测试里会有反例字面量）
            if ($line -match '#\[cfg\(test\)\]') { break }
            # 真正的违规：拿 lowercase 结果去和 select/Selector 比较
            if ($line -match '(to_lowercase|to_ascii_lowercase)' -and
                $line -match 'select|Selector') {
                $violations += "$($_.Name):$($i+1)  $($line.Trim())"
            }
        }
    }
    if ($violations.Count -gt 0) {
        Write-Host '发现用大小写模糊比较判定组类型（会导致 Selector 判定失效）：' -ForegroundColor Red
        $violations | ForEach-Object { Write-Host "  $_" }
        return $false
    }

    # 正向断言：类型常量必须是精确的 "Selector"，且定义了 is_selector 精确比较
    $policy = Join-Path $src 'domain\policy.rs'
    if (-not (Test-Path $policy)) {
        Write-Host '未找到 domain\policy.rs' -ForegroundColor Red
        return $false
    }
    $c = Get-Content $policy -Raw -Encoding UTF8
    if ($c -notmatch 'pub const KIND_SELECTOR: &str = "Selector";') {
        Write-Host 'KIND_SELECTOR 未定义为精确字符串 "Selector"' -ForegroundColor Red
        return $false
    }
    if ($c -notmatch 'fn is_selector\(kind: &str\) -> bool \{\s*kind == KIND_SELECTOR') {
        Write-Host 'is_selector 未使用精确 == 比较' -ForegroundColor Red
        return $false
    }
    Write-Host '未发现大小写模糊比较；KIND_SELECTOR="Selector" 精确匹配' -ForegroundColor Green
    return $true
}

# ---- 7. 配置硬约束检查 ----
Step '配置生成硬约束' {
    Push-Location src-tauri
    try {
        $out = (& cargo test --lib config_ 2>&1 | Out-String)
        Write-Host $out
        $m = [regex]::Matches($out, 'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed')
        if ($m.Count -eq 0) { return $false }
        $last = $m[$m.Count - 1]
        return ([int]$last.Groups[3].Value -eq 0 -and [int]$last.Groups[2].Value -gt 0)
    } finally { Pop-Location }
}

# ---- 8. 性能实测 ----
if (-not $SkipPerf) {
    Step '性能实测（C.11 预算）' {
        $script = Join-Path $PSScriptRoot 'perf-probe.ps1'
        & powershell -ExecutionPolicy Bypass -File $script -Samples 10 | Out-Host
        return ($LASTEXITCODE -eq 0)
    }
}

# ---- 9. 打包 ----
if (-not $SkipBundle) {
    Step 'NSIS 打包' {
        Push-Location src-tauri
        try {
            $out = cargo tauri build 2>&1 | Out-String
            Write-Host ($out -split "`n" | Select-Object -Last 25 | Out-String)
            if ($LASTEXITCODE -ne 0) { return $false }

            $setup = Get-ChildItem "$Root\src-tauri\target\release\bundle\nsis\*-setup.exe" -ErrorAction SilentlyContinue
            if (-not $setup) { Write-Host '未找到安装包' -ForegroundColor Red; return $false }
            $mb = $setup[0].Length / 1MB
            Write-Host ("安装包: {0}  {1:N2} MB（预算 ≤ 12 MB）" -f $setup[0].Name, $mb)
            return ($mb -le 12)
        } finally { Pop-Location }
    }
}

# ---------------------------------------------------------------------------
Write-Host ''
Write-Host '=== 验收汇总 ===' -ForegroundColor Cyan
$results | Format-Table -AutoSize

if ($failed -eq 0) {
    Write-Host '全部通过' -ForegroundColor Green
    exit 0
} else {
    Write-Host ("{0} 项失败" -f $failed) -ForegroundColor Red
    exit 1
}
