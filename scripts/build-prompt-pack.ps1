<#
.SYNOPSIS
    把 prompts/ 目录下的全部章节按投喂顺序拼装成一个单文件 prompt-pack.md。

.DESCRIPTION
    用途：一次性把完整提示词套件投喂给 AI 编码代理（超长上下文模式）。
    顺序由 $Order 定义，未列出的 .md 会被追加到末尾并告警，方便发现漏登记的新章节。

.EXAMPLE
    pwsh -File .\scripts\build-prompt-pack.ps1
    pwsh -File .\scripts\build-prompt-pack.ps1 -Output .\dist\pack.md -IncludeReadme:$false
#>
[CmdletBinding()]
param(
    [string]$PromptRoot,
    [string]$Output,
    [switch]$IncludeReadme
)

$ErrorActionPreference = 'Stop'

# 解析根目录与输出路径（PS 5.1 中 $PSScriptRoot 在 param 默认值里不可用）
$scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
if (-not $PromptRoot) { $PromptRoot = Join-Path $scriptDir '..\prompts' }
if (-not $Output) { $Output = Join-Path $scriptDir '..\prompt-pack.md' }

$Order = @(
    '01-master-prompt.md'
    'appendices\C-conventions.md'
    '02-tech-stack-and-architecture.md'
    '03-subscription-fetch-and-parse.md'
    '04-multi-subscription-merge.md'
    '05-scheduler-and-auto-update.md'
    '06-speedtest-engine.md'
    '07-auto-switch-policy.md'
    '08-kernel-and-system-proxy.md'
    '09-ui-ux-and-tray.md'
    '10-lightweight-and-performance.md'
    '11-packaging-and-release.md'
    '12-testing-and-acceptance.md'
    '13-iteration-task-cards.md'
    'appendices\A-mihomo-api-and-config.md'
    'appendices\B-subscription-userinfo.md'
)

if (-not (Test-Path -LiteralPath $PromptRoot)) {
    throw "找不到提示词目录：$PromptRoot"
}

$missing = @()
foreach ($rel in $Order) {
    $full = Join-Path $PromptRoot $rel
    if (-not (Test-Path -LiteralPath $full)) { $missing += $rel }
}
if ($missing.Count -gt 0) {
    throw "以下章节缺失，无法打包：`n  - $($missing -join "`n  - ")"
}

$sb = [System.Text.StringBuilder]::new()
$stamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz'
$version = 'v1.0'

[void]$sb.AppendLine("# Clash Zhengl · 完整开发提示词包（单文件版）")
[void]$sb.AppendLine()
[void]$sb.AppendLine("> 生成时间：$stamp  ")
[void]$sb.AppendLine("> 生成命令：``pwsh -File .\scripts\build-prompt-pack.ps1``  ")
[void]$sb.AppendLine('> 使用方式：把本文档全文作为一次性上下文投喂给 AI 编码代理，')
[void]$sb.AppendLine('> 然后在结尾追加一句：')
[void]$sb.AppendLine('>')
[void]$sb.AppendLine('> ```text')
[void]$sb.AppendLine('> 以上是你的完整工程契约与全部章节。请先按 01 §9 的模板回复确认，')
[void]$sb.AppendLine('> 然后执行 13 章中的 P0 任务卡；P0 完成后停下来等我验收，不要继续 P1。')
[void]$sb.AppendLine('> ```')
[void]$sb.AppendLine()
[void]$sb.AppendLine('---')
[void]$sb.AppendLine()

$index = 0
$total = $Order.Count
foreach ($rel in $Order) {
    $index++
    $full = Join-Path $PromptRoot $rel
    $text = Get-Content -LiteralPath $full -Raw -Encoding UTF8
    if ($null -eq $text) { $text = '' }

    [void]$sb.AppendLine("<!-- ===================== [$index/$total] $rel ===================== -->")
    [void]$sb.AppendLine()
    [void]$sb.AppendLine($text.TrimEnd())
    [void]$sb.AppendLine()
    [void]$sb.AppendLine('---')
    [void]$sb.AppendLine()
}

# 未在 Order 中登记的文件，追加到末尾并告警
$all = Get-ChildItem -LiteralPath $PromptRoot -Recurse -Filter '*.md' -File |
    Where-Object { $_.Name -ne '00-README.md' }
$registered = $Order | ForEach-Object { (Join-Path $PromptRoot $_) } | ForEach-Object { [System.IO.Path]::GetFullPath($_) }

$extra = @()
foreach ($f in $all) {
    if ($registered -notcontains [System.IO.Path]::GetFullPath($f.FullName)) { $extra += $f }
}
if ($IncludeReadme) {
    $readme = Join-Path $PromptRoot '00-README.md'
    if (Test-Path -LiteralPath $readme) { $extra = @(Get-Item -LiteralPath $readme) + $extra }
}
if ($extra.Count -gt 0) {
    Write-Warning "发现未登记章节，已追加到末尾（请把它们加入 `$Order 以免顺序不确定）："
    foreach ($f in $extra) { Write-Warning ("  - " + $f.FullName) }

    foreach ($f in $extra) {
        $text = Get-Content -LiteralPath $f.FullName -Raw -Encoding UTF8
        [void]$sb.AppendLine("<!-- ===================== [extra] $($f.Name) ===================== -->")
        [void]$sb.AppendLine()
        [void]$sb.AppendLine($text.TrimEnd())
        [void]$sb.AppendLine()
        [void]$sb.AppendLine('---')
        [void]$sb.AppendLine()
    }
}

$outDir = Split-Path -Parent $Output
if ($outDir -and -not (Test-Path -LiteralPath $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}

$utf8 = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($Output), $sb.ToString(), $utf8)

$chars = $sb.Length
$lines = ($sb.ToString() -split "`n").Count
Write-Host ''
Write-Host '打包完成' -ForegroundColor Green
Write-Host ("  输出文件 : " + [System.IO.Path]::GetFullPath($Output))
Write-Host ("  章节数量 : " + $total + $(if ($extra.Count) { " + $($extra.Count) extra" } else { '' }))
Write-Host ("  总行数   : " + $lines)
Write-Host ("  总字符   : " + $chars)
Write-Host ("  预估token: 约 " + [math]::Round($chars / 1.6) + "（中文粗估，仅供参考）")
Write-Host ''
Write-Host '提示：若目标模型上下文不足，请改用 README 的「方式 B 分阶段模式」。' -ForegroundColor Yellow
