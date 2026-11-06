<#
.SYNOPSIS
    契约一致性校验：检查各章节是否引用了 C 章（宪法）未登记的 IPC 命令 / 事件名 / 错误码，
    以及章节文件是否齐全、是否有标准的结构小标题。

.DESCRIPTION
    为什么需要它：
      提示词套件由多人（或多个 AI 会话）并行编写，最容易出的问题是**各章自己发明命令名与事件名**。
      一旦下游 AI 照着写了，就会产出一堆对不上的接口。本脚本把 C 章当作唯一真源做交叉核对。

    检查项：
      [1] 章节齐全性      —— 按 00-README 的清单核对文件是否存在
      [2] 结构完整性      —— 每章是否有「用途/适用阶段」开头与「自检清单」结尾
      [3] IPC 命令白名单  —— 反引号内的 `<domain>_<action>` 必须在 C.6 表中
      [4] 事件白名单      —— 反引号内的 `<domain>:<event>` 必须在 C.7 表中
      [5] 错误码白名单    —— `E_XXX` 必须在 C.8 表中
      [6] 单文件包新鲜度  —— prompt-pack.md 是否比所有章节旧（若存在）

    退出码：0 = 全部通过（仅告警也算通过）；1 = 存在硬错误。

.EXAMPLE
    pwsh -File .\scripts\check-contract.ps1
    pwsh -File .\scripts\check-contract.ps1 -Verbose -SkipStructure
#>
[CmdletBinding()]
param(
    [string]$PromptRoot,
    [switch]$SkipStructure,
    [string]$ReportPath
)

$ErrorActionPreference = 'Stop'

# 解析根目录（PS 5.1 中 $PSScriptRoot 在 param 默认值里不可用）
if (-not $PromptRoot) {
    $scriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
    $PromptRoot = Join-Path $scriptDir '..\prompts'
}

$C_PATH = Join-Path $PromptRoot 'appendices\C-conventions.md'
if (-not (Test-Path -LiteralPath $C_PATH)) { throw "找不到契约文件：$C_PATH" }

# 必须存在的章节（与 00-README §2 表格一致）
$REQUIRED = @(
    '00-README.md'
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

# IPC 命令的合法域前缀（用于「疑似命令名但未登记」的启发式判定，避免把 snake_case 变量名当命令）
$IPC_PREFIXES = @(
    'app', 'settings', 'subscription', 'traffic', 'speedtest', 'switch',
    'core', 'sysproxy', 'profile', 'logs', 'diagnostics', 'update'
)

$errors = [System.Collections.Generic.List[string]]::new()
$warnings = [System.Collections.Generic.List[string]]::new()

function Read-Md {
    param([string]$Path)
    return (Get-Content -LiteralPath $Path -Raw -Encoding UTF8)
}

# ---------------- 解析 C 章白名单 ----------------
$cText = Read-Md $C_PATH

function Get-BacktickedFromTable {
    param([string]$Text, [string]$SectionHeader, [string]$SectionEndHeader, [regex]$Pattern)
    $start = $Text.IndexOf($SectionHeader)
    if ($start -lt 0) { return @() }
    $end = if ($SectionEndHeader) { $Text.IndexOf($SectionEndHeader, $start) } else { -1 }
    $block = if ($end -gt $start) { $Text.Substring($start, $end - $start) } else { $Text.Substring($start) }
    $found = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($m in [regex]::Matches($block, '`([^`]+)`')) {
        $v = $m.Groups[1].Value.Trim()
        if ($Pattern.IsMatch($v)) { [void]$found.Add($v) }
    }
    return @($found)
}

# C.6 IPC 命令：形如 subscription_list / settings_save
$cmdPattern = [regex]'^[a-z][a-z0-9]*(_[a-z0-9]+)+$'
$knownCmds = Get-BacktickedFromTable -Text $cText -SectionHeader '## C.6' -SectionEndHeader '## C.7' -Pattern $cmdPattern
# C.6 表中一行可能有 `settings_get` / `settings_save` 两格，逐个提取即可

# C.7 事件：形如 core:state / speedtest:progress
$evtPattern = [regex]'^[a-z][a-z0-9-]*:[a-z][a-z0-9-]*$'
$knownEvents = Get-BacktickedFromTable -Text $cText -SectionHeader '## C.7' -SectionEndHeader '## C.8' -Pattern $evtPattern

# C.8 错误码：E_XXX
$errPattern = [regex]'^E_[A-Z0-9_]+$'
$knownCodes = Get-BacktickedFromTable -Text $cText -SectionHeader '## C.8' -SectionEndHeader '## C.9' -Pattern $errPattern

Write-Host ''
Write-Host '== 契约白名单 ==' -ForegroundColor Cyan
Write-Host ("  IPC 命令  : {0}" -f ($knownCmds -join ', '))
Write-Host ("  事件      : {0}" -f ($knownEvents -join ', '))
Write-Host ("  错误码    : {0}" -f ($knownCodes -join ', '))

if ($knownCmds.Count -eq 0) { $errors.Add('无法从 C.6 解析出任何 IPC 命令，契约文件格式可能被破坏') }
if ($knownEvents.Count -eq 0) { $errors.Add('无法从 C.7 解析出任何事件名，契约文件格式可能被破坏') }
if ($knownCodes.Count -eq 0) { $errors.Add('无法从 C.8 解析出任何错误码，契约文件格式可能被破坏') }

# ---------------- [1] 章节齐全性 ----------------
Write-Host ''
Write-Host '== [1] 章节齐全性 ==' -ForegroundColor Cyan
foreach ($rel in $REQUIRED) {
    $full = Join-Path $PromptRoot $rel
    if (-not (Test-Path -LiteralPath $full)) {
        $errors.Add("缺少章节文件：$rel")
        Write-Host ("  MISSING  {0}" -f $rel) -ForegroundColor Red
    } else {
        $lines = (Get-Content -LiteralPath $full -Encoding UTF8).Count
        Write-Host ("  OK       {0}  ({1} 行)" -f $rel, $lines)
    }
}

# 反向检查：存在但未登记的 md
$allMd = Get-ChildItem -LiteralPath $PromptRoot -Recurse -Filter '*.md' -File
$requiredFull = $REQUIRED | ForEach-Object { [System.IO.Path]::GetFullPath((Join-Path $PromptRoot $_)) }
foreach ($f in $allMd) {
    if ($requiredFull -notcontains [System.IO.Path]::GetFullPath($f.FullName)) {
        $warnings.Add("存在未在脚本清单登记的章节（请同步 00-README 与本脚本）：$($f.Name)")
        Write-Host ("  EXTRA    {0}" -f $f.Name) -ForegroundColor Yellow
    }
}

# ---------------- [2] 结构完整性 ----------------
if (-not $SkipStructure) {
    Write-Host ''
    Write-Host '== [2] 结构完整性 ==' -ForegroundColor Cyan
    foreach ($rel in $REQUIRED) {
        if ($rel -eq '00-README.md') { continue }
        $full = Join-Path $PromptRoot $rel
        if (-not (Test-Path -LiteralPath $full)) { continue }
        $t = Read-Md $full
        $isAppendix = $rel -like 'appendices\*'
        $isConvention = $rel -eq 'appendices\C-conventions.md'
        $head = $t.Substring(0, [math]::Min(1200, $t.Length))

        # 开头应说明用途/适用阶段/前置
        if ($head -notmatch '(用途|适用阶段|本章用途|权威边界|必读前置|适用版本|定位|本章定义|本章覆盖|用法|本章产出|执行入口)') {
            $warnings.Add("$rel 开头缺少「用途/适用阶段」说明")
            Write-Host ("  WARN     {0} 开头缺用途说明" -f $rel) -ForegroundColor Yellow
        }
        # 结尾应有自检/验收（C 章是契约本身、01 章是角色设定，均免检）
        if (-not $isConvention -and $rel -ne '01-master-prompt.md') {
            # 自检小节可能不在文件最后（后面还有对齐记录/附录），所以全文搜索而不是只看尾部
            if ($t -notmatch '(本章自检清单|自检清单|验收清单|交付清单|本章验收|验收标准|质量门禁|核对)') {
                $warnings.Add("$rel 缺少「自检清单/验收」小节")
                Write-Host ("  WARN     {0} 缺自检清单" -f $rel) -ForegroundColor Yellow
            }
        }
    }
}

# ---------------- [3][4][5] 引用白名单核对 ----------------
# 允许的「已知设置字段 / 结构字段」：这些是 snake_case，但属于 C.5 数据模型字段，不是 IPC 命令。
# 从 C 章代码块里自动收集 `pub <name>:` 与 `pub enum/struct` 名称，避免手工维护。
$fieldNames = [System.Collections.Generic.HashSet[string]]::new()
foreach ($m in [regex]::Matches($cText, 'pub\s+([a-z][a-z0-9_]*)\s*:')) {
    [void]$fieldNames.Add($m.Groups[1].Value)
}
foreach ($m in [regex]::Matches($cText, 'pub\s+(?:struct|enum)\s+([A-Za-z][A-Za-z0-9_]*)')) {
    $n = $m.Groups[1].Value
    # PascalCase → snake_case，便于匹配文档里提到的 `speedtest_timeout_ms` 形式
    $snake = [regex]::Replace($n, '(?<!^)([A-Z])', '_$1').ToLowerInvariant()
    [void]$fieldNames.Add($snake)
}
# 从 C.2 目录树收集模块名：`subscription/manager.rs` → `subscription_manager`（是模块，不是 IPC 命令）
foreach ($m in [regex]::Matches($cText, '([a-z][a-z0-9_]*)/([a-z][a-z0-9_]*)\.rs')) {
    [void]$fieldNames.Add($m.Groups[1].Value + '_' + $m.Groups[2].Value)
}
# 从 C.2 目录树收集「目录/文件.rs」的显式组合（`subscription/manager.rs` → `subscription_manager`）。
# 目录树是缩进结构，目录与文件名通常不在同一行，所以要按缩进栈解析父目录。
# 注意：**不做目录×文件的笛卡尔积**——那会把豁免放得太宽，导致真正的错误命令名被漏掉。
$c2Start = $cText.IndexOf('## C.2')
$c2End = $cText.IndexOf('## C.3')
$treeBlock = if ($c2Start -ge 0 -and $c2End -gt $c2Start) { $cText.Substring($c2Start, $c2End - $c2Start) } else { '' }
$dirStack = [System.Collections.Generic.List[object]]::new()   # (Indent, Name)
foreach ($line in ($treeBlock -split "`n")) {
    # 名称起始位置 = 该行中第一个「非制表符 / 非 ├ └ │ ─ 空格」字符的索引
    $mm = [regex]::Match($line, '^[│├└─\s]*([A-Za-z0-9_.\-]+)(/?)(.*)$')
    if (-not $mm.Success) { continue }
    $indent = $mm.Groups[1].Index
    $nm = $mm.Groups[1].Value
    $isDir = $mm.Groups[2].Value -eq '/'
    if ($isDir) {
        while ($dirStack.Count -gt 0 -and $dirStack[$dirStack.Count - 1].Indent -ge $indent) {
            $dirStack.RemoveAt($dirStack.Count - 1)
        }
        $dirStack.Add([pscustomobject]@{ Indent = $indent; Name = $nm })
        [void]$fieldNames.Add($nm)
    } elseif ($nm -match '^([a-z][a-z0-9_]*)\.rs$') {
        $file = $Matches[1]
        [void]$fieldNames.Add($file)
        # 归属最近的、缩进更小的目录（通常就是父目录）
        for ($k = $dirStack.Count - 1; $k -ge 0; $k--) {
            if ($dirStack[$k].Indent -lt $indent) {
                [void]$fieldNames.Add($dirStack[$k].Name + '_' + $file)
                break
            }
        }
    }
}
Write-Host ("  （自动收集到 {0} 个数据模型字段/模块名，用于排除误报）" -f $fieldNames.Count) -ForegroundColor DarkGray

function Test-Content {
    param([string]$RelPath)
    $full = Join-Path $PromptRoot $RelPath
    if (-not (Test-Path -LiteralPath $full)) { return }
    $t = Read-Md $full
    $name = [System.IO.Path]::GetFileName($RelPath)

    $cmdSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$knownCmds)
    $evtSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$knownEvents)
    $codeSet = [System.Collections.Generic.HashSet[string]]::new([string[]]$knownCodes)

    # 本文件内声明的 Rust 函数名（`fn xxx(` / `pub async fn xxx(`）——函数不是 IPC 命令，豁免
    $localFns = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($m in [regex]::Matches($t, '(?:pub\s+)?(?:async\s+)?fn\s+([a-z][a-z0-9_]*)')) {
        [void]$localFns.Add($m.Groups[1].Value)
    }
    # 本文件内的字符串 ID（菜单 id、事件总线 key 等，形如 with_id(app, "update_all"）——不是 IPC 命令
    $localIds = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($m in [regex]::Matches($t, 'with_id\([^,]+,\s*"([a-z][a-z0-9_]*)"')) {
        [void]$localIds.Add($m.Groups[1].Value)
    }
    # 文档里显式声明「契约缺口 / 未登记 / 不在 C.6」时，其**邻近行**中的名字降级为告警。
    # 只认邻近行（±3 行）而不是整个文件，否则一个文件里提过一句「契约缺口」就会漏掉所有真错误。
    $ackRe = '(契约缺口|未登记|不在 C\.6|不在命令表|待补契约|CONTRACT-CHANGE-REQUEST)'
    function Test-Acknowledged {
        param([string]$Text, [int]$MatchIndex)
        $ls = $Text.LastIndexOf("`n", [math]::Max(0, $MatchIndex - 1))
        if ($ls -lt 0) { $ls = 0 }
        for ($i = 0; $i -lt 3; $i++) {
            $ls = $Text.LastIndexOf("`n", [math]::Max(0, $ls - 1))
            if ($ls -lt 0) { $ls = 0; break }
        }
        $le = $MatchIndex
        for ($i = 0; $i -lt 3; $i++) {
            $n = $Text.IndexOf("`n", $le + 1)
            if ($n -lt 0) { $le = $Text.Length; break }
            $le = $n
        }
        $window = $Text.Substring($ls, [math]::Min($le - $ls, $Text.Length - $ls))
        return ($window -match $ackRe)
    }

    # 表格行里同时出现「已登记命令」→ 该行在做「UI 名称 ↔ 命令」映射，行内其他名字豁免
    $mappingRows = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($line in ($t -split "`n")) {
        if ($line -notmatch '^\s*\|') { continue }
        $hasCmd = $false
        foreach ($m in [regex]::Matches($line, '`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`')) {
            if ($cmdSet.Contains($m.Groups[1].Value)) { $hasCmd = $true; break }
        }
        if ($hasCmd) {
            foreach ($m in [regex]::Matches($line, '`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`')) {
                [void]$mappingRows.Add($m.Groups[1].Value)
            }
        }
    }

    # 3) IPC 命令
    foreach ($m in [regex]::Matches($t, '`([a-z][a-z0-9]*(?:_[a-z0-9]+)+)`')) {
        $v = $m.Groups[1].Value
        $domain = $v.Split('_')[0]
        if ($IPC_PREFIXES -notcontains $domain) { continue }      # 不是命令域，跳过（如 serde 字段名）
        if ($cmdSet.Contains($v)) { continue }
        if ($fieldNames.Contains($v)) { continue }                # C.5 数据模型字段，不是命令
        if ($localFns.Contains($v)) { continue }                  # 本文件声明的函数
        if ($localIds.Contains($v)) { continue }                  # 菜单/控件 id
        if ($mappingRows.Contains($v)) { continue }               # UI 名称 ↔ 命令 映射行
        # 形如 `selector::update_failure_counters` 或 `update_failure_counters()` —— 是函数引用，不是命令
        $before = if ($m.Index -ge 3) { $t.Substring($m.Index - 3, 3) } else { '' }
        $after = if ($m.Index + $m.Length + 2 -le $t.Length) { $t.Substring($m.Index + $m.Length, 2) } else { '' }
        if ($before.Contains('::') -or $after.StartsWith('()')) { continue }
        # 该 token 全文都以「函数调用」形式出现 → 是函数，不是命令
        $bare = [regex]::Escape($v)
        if (([regex]::Matches($t, "(?<![A-Za-z0-9_`])$bare\s*\(")).Count -gt 0 -and
            ([regex]::Matches($t, "(?<![A-Za-z0-9_`])$bare\s*[(]")).Count -gt 0) { continue }
        if (Test-Acknowledged -Text $t -MatchIndex $m.Index) { continue }
        # 允许「前缀泛指」写法（例如 subscription_ 泛指一族命令）
        $prefixHit = @($knownCmds | Where-Object { $_.StartsWith($v + '_') })
        if ($prefixHit.Count -gt 0) { continue }
        # 允许「命令名的前缀片段」（例如 core_version 是 core_versions_list 的词根）→ 仅告警不报错
        $softHit = @($knownCmds | Where-Object { $_.StartsWith($v) -or $v.StartsWith($_) })
        if ($softHit.Count -gt 0) {
            $warnings.Add("[$name] `$v` 疑似命令名片段，最接近的已登记命令：$($softHit -join ', ')")
            continue
        }
        if (Test-Acknowledged -Text $t -MatchIndex $m.Index) {
            $warnings.Add("[$name] `$v` 未在 C.6 登记（文档已声明契约缺口，确认是否要补契约）")
            continue
        }
        $errors.Add("[$name] 引用了 C.6 未登记的命令：$v")
        Write-Host ("  BAD-CMD  {0} → {1}" -f $name, $v) -ForegroundColor Red
    }

    # 4) 事件（限定在已知域前缀内，避免把 URL scheme / CSS 伪类当成事件）
    # 「C.7 没有 xxx 事件，不新增」「有无自造 switch:hold」这类否定/自检语境不算违规引用
    $evtDomains = @('core', 'subscription', 'traffic', 'speedtest', 'switch', 'sysproxy', 'notify')
    $negRe = '(没有|不存在|不新增|未定义|不新增该|无此|不使用|自造|未登记|禁止|是否只|不得|不许|反例|错误写法)'
    foreach ($m in [regex]::Matches($t, '`([a-z][a-z0-9-]*:[a-z][a-z0-9-]*)`')) {
        $v = $m.Groups[1].Value
        $d = $v.Split(':')[0]
        if ($evtDomains -notcontains $d) { continue }
        if ($evtSet.Contains($v)) { continue }
        # 取该匹配所在行的上下文，判断是否为否定语境
        $lineStart = $t.LastIndexOf("`n", [math]::Max(0, $m.Index - 1))
        $lineEnd = $t.IndexOf("`n", $m.Index)
        if ($lineStart -lt 0) { $lineStart = 0 }
        if ($lineEnd -lt 0) { $lineEnd = $t.Length }
        $line = $t.Substring($lineStart, $lineEnd - $lineStart)
        if ($line -match $negRe) { continue }
        $errors.Add("[$name] 引用了 C.7 未登记的事件：$v")
        Write-Host ("  BAD-EVT  {0} → {1}" -f $name, $v) -ForegroundColor Red
    }

    # 5) 错误码
    foreach ($m in [regex]::Matches($t, '\b(E_[A-Z][A-Z0-9_]+)\b')) {
        $v = $m.Groups[1].Value
        if ($codeSet.Contains($v)) { continue }
        # `E_NET_` / `E_STORE_` 这类是「一族错误码」的前缀写法，允许
        if ($v.EndsWith('_')) {
            $fam = @($knownCodes | Where-Object { $_.StartsWith($v) })
            if ($fam.Count -gt 0) { continue }
        }
        # `E_XXX_YYY` 这类是模板占位符，允许（整串必须只由 X/Y/Z 占位段构成）
        if ($v -match '^E_[XYZ]+(_[XYZ]+)*$') { continue }
        $errors.Add("[$name] 引用了 C.8 未登记的错误码：$v")
        Write-Host ("  BAD-CODE {0} → {1}" -f $name, $v) -ForegroundColor Red
    }
}

Write-Host ''
Write-Host '== [3][4][5] 命令 / 事件 / 错误码引用核对 ==' -ForegroundColor Cyan
$before = $errors.Count
foreach ($rel in $REQUIRED) {
    if ($rel -eq 'appendices\C-conventions.md') { continue }   # 真源自身豁免
    Test-Content $rel
}
if ($errors.Count -eq $before) { Write-Host '  全部引用均已在契约中登记' -ForegroundColor Green }

# ---------------- [6] 单文件包新鲜度 ----------------
Write-Host ''
Write-Host '== [6] 单文件包新鲜度 ==' -ForegroundColor Cyan
$packPath = Join-Path (Split-Path -Parent $PromptRoot) 'prompt-pack.md'
if (Test-Path -LiteralPath $packPath) {
    $packTime = (Get-Item -LiteralPath $packPath).LastWriteTime
    $newest = ($allMd | Sort-Object LastWriteTime -Descending | Select-Object -First 1)
    if ($newest -and $newest.LastWriteTime -gt $packTime) {
        $warnings.Add("prompt-pack.md 已过期（最新章节 $($newest.Name) 修改于 $($newest.LastWriteTime)，包生成于 $packTime），请重跑 build-prompt-pack.ps1")
        Write-Host ("  STALE    package 比 {0} 旧" -f $newest.Name) -ForegroundColor Yellow
    } else {
        Write-Host '  OK       prompt-pack.md 是最新的'
    }
} else {
    Write-Host '  跳过：尚未生成 prompt-pack.md'
}

# ---------------- 汇总 ----------------
Write-Host ''
Write-Host '== 汇总 ==' -ForegroundColor Cyan
Write-Host ("  错误 {0} 条 / 告警 {1} 条" -f $errors.Count, $warnings.Count)

if ($errors.Count -gt 0) {
    Write-Host ''
    Write-Host '错误清单：' -ForegroundColor Red
    foreach ($e in $errors) { Write-Host ("  - " + $e) -ForegroundColor Red }
}
if ($warnings.Count -gt 0) {
    Write-Host ''
    Write-Host '告警清单：' -ForegroundColor Yellow
    foreach ($w in $warnings) { Write-Host ("  - " + $w) -ForegroundColor Yellow }
}

if ($ReportPath) {
    $md = [System.Text.StringBuilder]::new()
    [void]$md.AppendLine('# 契约一致性校验报告')
    [void]$md.AppendLine()
    [void]$md.AppendLine(("- 生成时间：{0}" -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss zzz')))
    [void]$md.AppendLine(("- 错误：{0} 条；告警：{1} 条" -f $errors.Count, $warnings.Count))
    [void]$md.AppendLine()
    [void]$md.AppendLine('## 错误')
    [void]$md.AppendLine()
    if ($errors.Count -eq 0) { [void]$md.AppendLine('无。') } else { foreach ($e in $errors) { [void]$md.AppendLine("- $e") } }
    [void]$md.AppendLine()
    [void]$md.AppendLine('## 告警')
    [void]$md.AppendLine()
    if ($warnings.Count -eq 0) { [void]$md.AppendLine('无。') } else { foreach ($w in $warnings) { [void]$md.AppendLine("- $w") } }
    $utf8 = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText([System.IO.Path]::GetFullPath($ReportPath), $md.ToString(), $utf8)
    Write-Host ("报告已写入：{0}" -f [System.IO.Path]::GetFullPath($ReportPath))
}

if ($errors.Count -gt 0) { exit 1 }
Write-Host ''
Write-Host '契约一致。' -ForegroundColor Green
exit 0
