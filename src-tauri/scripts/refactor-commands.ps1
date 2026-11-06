# 批量把 Tauri 命令重构为「_inner 纯函数 + 薄 wrapper」。
#
# 目标形态：
#   pub async fn foo_inner(s: &Arc<AppState>, ...) -> AppResult<T> { <原函数体> }
#   #[tauri::command]
#   pub async fn foo(state: State<'_, AppHandleState>, ...) -> R<T> {
#       foo_inner(&state.0, ...).await.map_err(IpcError::from)
#   }
#
# 这样 daemon 与 Tauri 共用同一实现，行为不可能漂移。

$ErrorActionPreference = "Stop"
$base = "C:\Data\DSH\clash zhengl\src-tauri"

$files = @(
    "src\commands\app_cmd.rs",
    "src\commands\sub_cmd.rs",
    "src\commands\node_cmd.rs"
)

foreach ($rel in $files) {
    $path = Join-Path $base $rel
    $text = [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8)
    $lines = $text -split "`r?`n", -1
    $out = New-Object System.Collections.Generic.List[string]
    $i = 0
    $converted = 0

    while ($i -lt $lines.Count) {
        $line = $lines[$i]

        if ($line -match '^\s*#\[tauri::command\]\s*$') {
            # ---- 解析函数签名（可能跨多行）----
            $j = $i + 1
            $sigLines = New-Object System.Collections.Generic.List[string]
            while ($j -lt $lines.Count -and $lines[$j] -notmatch '\{') {
                $sigLines.Add($lines[$j])
                $j++
            }
            $openLine = $lines[$j]
            $sigLines.Add(($openLine -split '\{')[0])
            $sigText = ($sigLines -join " ") -replace '\s+', ' '

            # 提取函数名
            if ($sigText -notmatch 'pub\s+(?:async\s+)?fn\s+(\w+)\s*\(') {
                $out.Add($line); $i++; continue
            }
            $fnName = $Matches[1]

            # 提取 State 之外的参数（跳过 state: State<...>）
            $paramBlock = ($sigText -replace '^.*?fn\s+\w+\s*\(', '') -replace '\)\s*->.*$', ''
            $params = New-Object System.Collections.Generic.List[string]
            $depth = 0; $cur = ""
            foreach ($ch in $paramBlock.ToCharArray()) {
                if ($ch -eq '<' -or $ch -eq '(') { $depth++ }
                elseif ($ch -eq '>' -or $ch -eq ')') { $depth-- }
                if ($ch -eq ',' -and $depth -eq 0) { $params.Add($cur); $cur = "" }
                else { $cur += $ch }
            }
            if ($cur.Trim()) { $params.Add($cur) }

            $extraParams = @()
            $extraArgs = @()
            foreach ($p in $params) {
                $pt = $p.Trim()
                if (-not $pt) { continue }
                if ($pt -match '^\s*state\s*:\s*State') { continue }
                if ($pt -match '^\s*app\s*:\s*tauri::AppHandle') { continue }
                $extraParams += $pt
                if ($pt -match '^(\w+)\s*:') { $extraArgs += $Matches[1] }
            }

            # 返回类型
            $retType = "AppResult<()>"
            if ($sigText -match '\)\s*->\s*R<(.+?)>\s*$') { $retType = "AppResult<" + $Matches[1] + ">" }
            $isAsync = $sigText -match 'pub\s+async\s+fn'

            # ---- 找函数体配对的右花括号 ----
            $depthBrace = 0
            $k = $j
            $bodyStart = $j
            $foundOpen = $false
            while ($k -lt $lines.Count) {
                foreach ($ch in $lines[$k].ToCharArray()) {
                    if ($ch -eq '{') { $depthBrace++; $foundOpen = $true }
                    elseif ($ch -eq '}') { $depthBrace-- }
                }
                if ($foundOpen -and $depthBrace -le 0) { break }
                $k++
            }
            $bodyEnd = $k

            # 函数体（去掉首尾花括号所在行）
            $bodyLines = @()
            if ($bodyEnd -eq $bodyStart) {
                # 单行函数
                $inner = ($lines[$bodyStart] -replace '^[^{]*\{', '') -replace '\}\s*$', ''
                $bodyLines = @($inner)
            } else {
                $firstPart = ($lines[$bodyStart] -replace '^[^{]*\{', '')
                if ($firstPart.Trim()) { $bodyLines += $firstPart }
                for ($m = $bodyStart + 1; $m -lt $bodyEnd; $m++) { $bodyLines += $lines[$m] }
                $lastPart = ($lines[$bodyEnd] -replace '\}\s*$', '')
                if ($lastPart.Trim()) { $bodyLines += $lastPart }
            }

            # ---- 生成 _inner ----
            $asyncKw = if ($isAsync) { "async " } else { "" }
            $innerParams = @("s: &Arc<crate::commands::state::AppState>") + $extraParams
            if ($asyncKw) {
                # async fn 的参数要加生命周期
            }
            $out.Add(("pub {0}fn {1}_inner({2}) -> {3} {{" -f $asyncKw, $fnName, ($innerParams -join ', '), $retType))
            foreach ($bl in $bodyLines) {
                # 把 st(&state) 改成直接使用 s；把 .map_err(IpcError::from)? 去掉
                $converted_line = $bl
                $converted_line = $converted_line -replace 'let s = st\(&state\);', ''
                $converted_line = $converted_line -replace '\.map_err\(IpcError::from\)\?', '?'
                $out.Add($converted_line)
            }
            $out.Add("}")
            $out.Add("")

            # ---- 生成 wrapper ----
            $wrapParams = @("state: State<'_, AppHandleState>") + $extraParams
            $callArgs = @("&state.0") + $extraArgs
            $awaitKw = if ($isAsync) { ".await" } else { "" }
            $wrapRet = "R<" + ($retType -replace '^AppResult<', '' -replace '>$', '') + ">"
            $out.Add("#[tauri::command]")
            $out.Add(("pub {0}fn {1}({2}) -> {3} {{" -f $asyncKw, $fnName, ($wrapParams -join ', '), $wrapRet))
            $out.Add(("    {1}_inner({0}){2}.map_err(IpcError::from)" -f ($callArgs -join ', '), $fnName, $awaitKw))
            $out.Add("}")
            $out.Add("")

            $converted++
            $i = $bodyEnd + 1
            continue
        }

        $out.Add($line)
        $i++
    }

    [System.IO.File]::WriteAllText($path, ($out -join "`r`n"), (New-Object System.Text.UTF8Encoding($false)))
    Write-Output "$rel : 转换 $converted 个命令"
}
