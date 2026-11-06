# 12 · 测试与验收

> **定位**：本章定义「什么叫做完」。任何阶段交付都必须能对上本章的某张表。
> **契约依赖**：[C-conventions.md](appendices/C-conventions.md) §C.6（IPC）、§C.7（事件）、§C.8（错误码）、§C.11（性能预算）、§C.13（质量门禁）。
> **契约版本**：对齐 **C 章 v1.4**（2026-11-06）。命令行/事件名/错误码白名单见 12.4.1；本章自检清单见文末。
> **铁律**：没测过的功能写「未验证」，不许写「应该没问题」（01 章 §8）。

---

## 12.1 测试分层与职责

| 层 | 范围 | 落点（C.2 路径） | 运行环境 | 目标数量 | 门禁 |
| --- | --- | --- | --- | --- | --- |
| L1 单元 | 纯函数：解析、排序、到期、退避、切换判定 | `src-tauri/src/**/*.rs` 内 `#[cfg(test)] mod tests` | 无需网络、无需内核 | ≥ 130 个 | `cargo test` 必绿 |
| L2 集成 | 订阅拉取 → 解析 → 合并 → 落盘；Mock 服务端到端 | `src-tauri/tests/` | 本地 Mock server | ≥ 25 个 | `cargo test --test '*'` |
| L3 契约 | IPC 命令名/入参/返回/错误码与 C.6/§C.8 一致 | `src-tauri/tests/contract_*.rs` + 前端 `src/ipc/client.ts` | 纯静态 + 反射式断言 | 全部命令 | 必绿 |
| L4 端到端 | 装包 → 启动 → 加订阅 → 开代理 → 访问外网 | 手工 + `scripts/e2e-smoke.ps1` | 真机 | 每阶段 1 轮 | 手工签字 |
| L5 手工/异常 | 断网、500、HTML、坏配置、杀内核、端口占用、代理残留、手动覆盖 | 手工清单（12.6） | 真机 | 28 条全过 | 手工签字 |
| L6 性能 | C.11 全部指标 | `scripts/perf-probe.ps1` | 真机 release | 10 项 | 数字达标 |
| L7 回归 | 上述 L1–L6 每阶段复跑 | CI + 本地 | — | — | 无红 |

> **分层铁律**：L1 不许碰网络与文件系统；L2 不许依赖真实内核；真实内核只在 L4/L5 出现。

---

## 12.2 测试矩阵

### 12.2.1 按模块 × 层

| 模块 | L1 单元 | L2 集成 | L3 契约 | L4 E2E | L5 异常 |
| --- | --- | --- | --- | --- | --- |
| `subscription/userinfo.rs` | 15 例（附录 B.7） | 头解析 → meta.json 落盘 | `subscription_refresh` 返回含 mgmt | 真实机场 1 条 | 无头 / 畸形头 |
| `subscription/fetch.rs` | 重试/退避计算、超时分类 | Mock 7 场景 | `E_NET_*` / `E_HTTP_STATUS` 映射 | 真实订阅 | 断网 / 500 / 慢速 / 断流 |
| `subscription/parse.rs` | 非法 YAML、空 proxies、类型统计 | 8 MB 大配置 | `E_PARSE_YAML` / `E_PARSE_EMPTY` | 真实配置 | HTML 当 YAML |
| `subscription/manager.rs` | 去重、冲突前缀 | 3 订阅合并 → runtime config | `subscription_*` CRUD | 3 条真实订阅 | 同名节点、重复 URL |
| `scheduler/due.rs` | 到期集合纯函数（含时间跳变） | 假时钟推进 | — | 15 分钟周期观察 | 系统时间回拨 |
| `scheduler/backoff.rs` | 退避序列 + 抖动范围 | 连续失败 5 次 | — | — | 一直失败 |
| `speedtest/rank.rs` | 排序键、排除规则、tolerance、手动锁定过滤 | 假结果集排序 | `switch_apply_best` | 真实节点 | 全失败 / 全 0ms |
| `speedtest/engine.rs` | 信号量并发度 | 假 API 200 节点 | `speedtest:*` 事件 | 真实 300+ 节点 | 取消 / 中途杀内核 |
| `speedtest/selector.rs` | 防抖三条件、`SwitchRuntimeLocks` 锁定跳过与到期 | PUT 204 幂等、`kind != "Selector"` 跳过 | `switch:applied` | 真实切换 | 组不存在 / 目标被拒 / 锁定中 |
| `kernel/supervisor.rs` | 端口探测 | 起停真内核 | `core_*` 幂等 | 冷启动 | 内核被 kill / 端口占用 |
| `kernel/api.rs` | URL 编码、状态码映射、**`kind == "Selector"` 大小写敏感**（12.4.2） | 401 / 404 / 504 | `E_CORE_API_UNAUTH` | — | secret 不一致 / kind 写法漂移 |
| `kernel/fetch_core.rs` | 资产名正则、SHA256 校验、`.part` 续传 | 假 HTTP 下载 | `core_install` + `core:install-progress` | 真实下载 | 断网 / 校验失败 / 磁盘满 |
| `system/sysproxy.rs` | 注册表值拼装、journal 快照 | 开→关→恢复（含崩溃路径） | `sysproxy_set` → `SysProxyState`；事件 `sysproxy:state` | 浏览器访问外网 | 崩溃后残留代理 / 被外力改动 |
| `store/json_store.rs` | 原子写、损坏回退 | tmp→rename | `settings_*` | 断电模拟 | 磁盘只读 |
| `store/crypto.rs` | 加解密往返 | 换用户解密失败 | `E_STORE_DECRYPT` | — | 密文被篡改 |
| `store/migrate.rs` | schema_version 1→2 迁移 | 旧 settings.json 可读 | `settings_get` 不报错 | — | 缺 `compliance_accepted_at` 字段 |
| `traffic` 聚合（`TrafficSummary`） | 聚合规则纯函数（C.5 注释） | 多订阅混合 total=0/缺失 | `traffic_summary` | 真实 3 订阅 | 全部缺失 / 全为不限量 |
| **配置生成器（`subscription/manager.rs` 的 `config_gen`）** | **配置取值回归断言（12.4.4）**：`find-process-mode`/`ipv6`/`dns.enable`/`proxy-groups[].type` | 解析生成的 YAML 后逐项比对 | `profile_preview` | 真内核 `-t` 自检 | 取值被"优化"改错 |
| 前端 `src/ipc/client.ts` | 错误归一 | — | 与 C.6 命令名逐一比对 | — | 后端抛错 → 中文提示 |

### 12.2.2 按阶段 × 必测层

| 阶段 | L1 | L2 | L3 | L4 | L5 | L6 |
| --- | --- | --- | --- | --- | --- | --- |
| P0 脚手架 | 1 个 smoke | — | 命令表空校验 | 启动到托盘 | — | M5 |
| P1 内核与系统代理 | 端口/net | 起停内核 | `core_*` / `sysproxy_set` | 浏览器走代理 | 杀内核 / 端口占用 | M1 M6 |
| P2 订阅与合并 | userinfo/parse | Mock 7 场景 | `subscription_*` | 真实 3 订阅 | 全部异常订阅 | 合并峰值 |
| P3 调度器 | due/backoff | 假时钟 | — | 15min 观察 | 时间跳变 | M3 M9 |
| P4 测速与切换 | rank/selector | 假 API | `speedtest_*` / `switch_*` | 真实测速切换 | 全失败 / 取消 | M4 |
| P5 UI 与托盘 | — | — | 事件名比对 | 三页面操作 | 空态/错误态 | M1 M2 |
| P6 轻量化 | — | — | — | — | — | 全指标 |
| P7 打包 | — | — | — | 安装/卸载 | 无 WebView2 | M7 M8 |
| P8 验收 | 全量 | 全量 | 全量 | 全量 | 28 条 | 全部 ×3 |

---

## 12.3 Mock 订阅服务器（PowerShell 7 完整实现）

> 落盘：`tests/mock-sub-server.ps1`。基于 `System.Net.HttpListener`，零依赖，pwsh 7 直跑。
> 覆盖 7 种场景，用路径前缀区分：`/ok` `/nouser` `/500` `/html` `/big` `/slow` `/broken`。

### 12.3.1 完整脚本

```powershell
<#
.SYNOPSIS
    Clash Zhengl 订阅 Mock 服务器（测试专用，禁止用于生产）。
.DESCRIPTION
    7 种场景：
      /ok         200 + 合法 Clash YAML + subscription-userinfo 头
      /nouser     200 + 合法 YAML，无 subscription-userinfo 头
      /500        500 Internal Server Error + 纯文本
      /html       200 + text/html（HTML 页面，不是 YAML）
      /big        200 + 约 8 MB 合法 YAML（5000 节点）
      /slow       200 + 合法 YAML，但先等 3 秒再响应
      /broken     200 + 声明 Content-Length 但只发一半就断开
      /junk       200 + 合法 YAML，但 userinfo 头为畸形值（负/字母/毫秒）
.EXAMPLE
    pwsh -File .\tests\mock-sub-server.ps1 -Port 18080
    # 然后订阅地址填 http://127.0.0.1:18080/ok
#>
[CmdletBinding()]
param(
    [int]$Port = 18080,
    [string]$BindAddress = '127.0.0.1',
    [int]$BigNodeCount = 5000,
    [switch]$Verbose
)

$ErrorActionPreference = 'Stop'

function New-SampleYaml {
    param([int]$NodeCount = 6, [string]$Prefix = 'Mock')
    $sb = [System.Text.StringBuilder]::new()
    [void]$sb.AppendLine('mixed-port: 10801')
    [void]$sb.AppendLine('mode: rule')
    [void]$sb.AppendLine('proxies:')
    $types = @('ss','vmess','trojan','vless','hysteria2')
    for ($i = 1; $i -le $NodeCount; $i++) {
        $t = $types[($i - 1) % $types.Count]
        [void]$sb.AppendLine("  - name: `"$Prefix-$($t.ToUpper())-$('{0:D4}' -f $i)`"")
        [void]$sb.AppendLine("    type: $t")
        [void]$sb.AppendLine("    server: 203.0.113.$((($i - 1) % 254) + 1)")
        [void]$sb.AppendLine("    port: $((10000 + $i) % 65535)")
        switch ($t) {
            'ss'         { [void]$sb.AppendLine('    cipher: aes-128-gcm'); [void]$sb.AppendLine('    password: "mockpass"') }
            'vmess'      { [void]$sb.AppendLine('    uuid: "b831381d-6324-4d53-ad4f-8cda48b30811"'); [void]$sb.AppendLine('    alterId: 0'); [void]$sb.AppendLine('    cipher: auto') }
            'trojan'     { [void]$sb.AppendLine('    password: "mockpass"') }
            'vless'      { [void]$sb.AppendLine('    uuid: "b831381d-6324-4d53-ad4f-8cda48b30811"') }
            'hysteria2'  { [void]$sb.AppendLine('    password: "mockpass"'); [void]$sb.AppendLine('    sni: mock.invalid') }
        }
        [void]$sb.AppendLine('    udp: true')
    }
    [void]$sb.AppendLine('proxy-groups:')
    [void]$sb.AppendLine('  - name: 节点选择')
    [void]$sb.AppendLine('    type: select')
    [void]$sb.AppendLine('    proxies: [自动选择, DIRECT]')
    [void]$sb.AppendLine('  - name: 自动选择')
    [void]$sb.AppendLine('    type: url-test')
    [void]$sb.AppendLine('    url: http://www.gstatic.com/generate_204')
    [void]$sb.AppendLine('    interval: 300')
    [void]$sb.AppendLine('    proxies:')
    for ($i = 1; $i -le $NodeCount; $i++) {
        $t = $types[($i - 1) % $types.Count]
        [void]$sb.AppendLine("      - `"$Prefix-$($t.ToUpper())-$('{0:D4}' -f $i)`"")
    }
    [void]$sb.AppendLine('rules:')
    [void]$sb.AppendLine('  - GEOIP,LAN,DIRECT,no-resolve')
    [void]$sb.AppendLine('  - MATCH,节点选择')
    return $sb.ToString()
}

function Write-Response {
    param($Ctx, [int]$Status, [string]$Body, [hashtable]$Headers = @{}, [string]$ContentType = 'text/plain; charset=utf-8')
    $res = $Ctx.Response
    $res.StatusCode = $Status
    $res.ContentType = $ContentType
    foreach ($k in $Headers.Keys) { $res.Headers[$k] = $Headers[$k] }
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Body)
    $res.ContentLength64 = $bytes.Length
    $res.OutputStream.Write($bytes, 0, $bytes.Length)
    $res.OutputStream.Flush()
    $res.Close()
}

$listener = [System.Net.HttpListener]::new()
$prefix = "http://${BindAddress}:$Port/"
$listener.Prefixes.Add($prefix)
try { $listener.Start() }
catch { throw "无法监听 $prefix —— 端口被占用或无权限：$($_.Exception.Message)" }

Write-Host "Mock 订阅服务器已启动：$prefix" -ForegroundColor Green
Write-Host @""
可用路径：
  GET /ok        200 合法 YAML + userinfo 头
  GET /nouser    200 合法 YAML，无 userinfo 头
  GET /500       500 纯文本错误
  GET /html      200 HTML 页面（伪装成 YAML）
  GET /big       200 约 8 MB / 5000 节点
  GET /slow      200 延迟 3 秒后返回
  GET /broken    200 声明长度但只发一半
  GET /junk      200 合法 YAML + 畸形 userinfo 头
按 Ctrl+C 停止。
"@

$bigYaml = $null   # 懒加载，避免启动慢

try {
    while ($listener.IsListening) {
        $ctx = $listener.GetContext()   # 阻塞等待
        $path = $ctx.Request.Url.AbsolutePath.TrimEnd('/').ToLowerInvariant()
        if ($Verbose) { Write-Host "→ $($ctx.Request.HttpMethod) $path" }
        try {
            switch ($path) {
                '/ok' {
                    $yaml = New-SampleYaml -NodeCount 6 -Prefix 'Mock'
                    Write-Response -Ctx $ctx -Status 200 -Body $yaml `
                        -ContentType 'text/yaml; charset=utf-8' `
                        -Headers @{
                            'subscription-userinfo' = 'upload=455727941; download=6174315081; total=1073741824000; expire=1798732800'
                            'profile-web-page-url'  = 'https://mock.invalid/user'
                            'profile-update-interval' = '24'
                        }
                }
                '/nouser' {
                    $yaml = New-SampleYaml -NodeCount 6 -Prefix 'NoUser'
                    Write-Response -Ctx $ctx -Status 200 -Body $yaml -ContentType 'text/yaml; charset=utf-8'
                }
                '/500' {
                    Write-Response -Ctx $ctx -Status 500 -Body 'Internal Server Error (mock)' `
                        -ContentType 'text/plain; charset=utf-8'
                }
                '/html' {
                    Write-Response -Ctx $ctx -Status 200 `
                        -Body '<!DOCTYPE html><html><head><title>Login</title></head><body><h1>请先登录</h1><p>Your subscription has expired.</p></body></html>' `
                        -ContentType 'text/html; charset=utf-8'
                }
                '/big' {
                    if ($null -eq $bigYaml) {
                        Write-Host "  生成 $BigNodeCount 节点大配置（首次请求，约 2~5 秒）..." -ForegroundColor Yellow
                        $bigYaml = New-SampleYaml -NodeCount $BigNodeCount -Prefix 'Big'
                    }
                    Write-Response -Ctx $ctx -Status 200 -Body $bigYaml -ContentType 'text/yaml; charset=utf-8'
                }
                '/slow' {
                    Write-Host '  /slow：等待 3 秒...' -ForegroundColor Yellow
                    Start-Sleep -Seconds 3
                    $yaml = New-SampleYaml -NodeCount 6 -Prefix 'Slow'
                    Write-Response -Ctx $ctx -Status 200 -Body $yaml -ContentType 'text/yaml; charset=utf-8' `
                        -Headers @{ 'subscription-userinfo' = 'upload=1; download=2; total=3; expire=1798732800' }
                }
                '/broken' {
                    # 声明一个较大 Content-Length，然后只写一半就关闭 → 模拟断流
                    $yaml = New-SampleYaml -NodeCount 200 -Prefix 'Broken'
                    $bytes = [System.Text.Encoding]::UTF8.GetBytes($yaml)
                    $res = $ctx.Response
                    $res.StatusCode = 200
                    $res.ContentType = 'text/yaml; charset=utf-8'
                    $res.ContentLength64 = $bytes.Length
                    $half = [int]($bytes.Length / 2)
                    $res.OutputStream.Write($bytes, 0, $half)
                    $res.OutputStream.Flush()
                    $res.Abort()                     # 关键：强制中断连接，不发送剩余字节
                    Write-Host '  /broken：已写入一半后中断' -ForegroundColor Yellow
                }
                '/junk' {
                    $yaml = New-SampleYaml -NodeCount 4 -Prefix 'Junk'
                    Write-Response -Ctx $ctx -Status 200 -Body $yaml -ContentType 'text/yaml; charset=utf-8' `
                        -Headers @{
                            'subscription-userinfo' = 'upload=-1; download=abc; total=0; expire=1798732800000; plan=pro'
                        }
                }
                default {
                    Write-Response -Ctx $ctx -Status 404 -Body 'not found'
                }
            }
        } catch {
            Write-Host "处理 $path 出错：$($_.Exception.Message)" -ForegroundColor Red
            try { $ctx.Response.Abort() } catch { }
        }
    }
}
finally {
    $listener.Stop()
    $listener.Close()
    Write-Host 'Mock 服务器已停止。' -ForegroundColor DarkGray
}
```

### 12.3.2 使用与验证

```powershell
# 终端 A：启动
pwsh -File .\tests\mock-sub-server.ps1 -Port 18080 -Verbose

# 终端 B：逐条验证场景
Invoke-WebRequest "http://127.0.0.1:18080/ok"     | Select-Object StatusCode, @{n='userinfo';e={$_.Headers['subscription-userinfo']}}
Invoke-WebRequest "http://127.0.0.1:18080/500"    -SkipHttpErrorCheck | Select-Object StatusCode
Invoke-WebRequest "http://127.0.0.1:18080/html"   | Select-Object StatusCode, @{n='ct';e={$_.Headers['Content-Type']}}

# 断流场景：应抛异常
try { Invoke-WebRequest "http://127.0.0.1:18080/broken" -TimeoutSec 30 } catch { "预期失败：$($_.Exception.Message)" }

# 大配置：核对体积
$r = Invoke-WebRequest "http://127.0.0.1:18080/big"
"big = {0:N2} MB" -f ($r.RawContentLength / 1MB)
```

**产品侧期望行为**（写进 L2 集成测试断言）：

| 路径 | 期望结果 | 错误码 | 落盘 |
| --- | --- | --- | --- |
| `/ok` | 成功，`mgmt` 四项有值，`source=Header` | — | yaml + meta.json 写入 |
| `/nouser` | 成功，`mgmt` 存在但四项 `None`，`source=Unknown` | 不报错（列表页显示「—」） | yaml + meta 写入 |
| `/500` | 失败，`last_update_status=Failed` | `E_HTTP_STATUS` | **不覆盖**旧 yaml |
| `/html` | 失败 | `E_PARSE_YAML` | 不覆盖旧 yaml |
| `/big` | 成功但超体积上限 → 拒绝 | `E_PARSE_YAML`（8 MB 边界按 10.4.5 上限表） | 不写入 |
| `/slow` | 若超 `sub_fetch_timeout_ms`（默认 15000）→ 成功；若设为 2000 → `E_NET_TIMEOUT` | `E_NET_TIMEOUT` | 视结果 |
| `/broken` | 失败 | `E_NET_TIMEOUT` 或 `E_HTTP_STATUS`（以实际读取错误映射，需在测试里固定一种并写入断言） | 不覆盖旧 yaml |
| `/junk` | 成功，`upload/download` 为 `None`，`total=Some(0)`，`expire=Some(1798732800)` | — | 写入 |

> **断流场景的错误码**：`reqwest` 读 body 中途 EOF 会报 `reqwest::Error`（`is_body()` / `is_decode()`），
> 统一映射为 `E_NET_TIMEOUT` 更贴近用户感受（「连接订阅服务器超时」）。**实现与测试必须约定同一个码**，
> 不要一个写 `E_NET_TIMEOUT` 一个写 `E_HTTP_STATUS`。

### 12.3.3 Rust 集成测试侧的最小 Mock（`cargo test` 内跑）

> PowerShell 服务器用于人工/手工验证；`cargo test` 里用进程内 server，避免依赖外部进程。

```rust
// src-tauri/tests/common/mock_server.rs
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

pub struct MockHandle { pub port: u16 }

/// 极简 HTTP/1.1 单次响应服务器：收到第一个请求后回固定内容，然后退出。
pub fn spawn_once(status: u16, headers: Vec<(&'static str, String)>, body: Vec<u8>) -> MockHandle {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);            // 忽略请求内容
            let mut resp = format!("HTTP/1.1 {} {}\r\n", status, reason(status));
            for (k, v) in &headers {
                resp.push_str(&format!("{k}: {v}\r\n"));
            }
            resp.push_str(&format!("Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()));
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    MockHandle { port }
}

fn reason(s: u16) -> &'static str {
    match s { 200 => "OK", 404 => "Not Found", 500 => "Internal Server Error", _ => "Unknown" }
}
```

```rust
// src-tauri/tests/fetch_scenarios.rs
mod common;
use common::mock_server::{spawn_once, MockHandle};

const MIN_YAML: &str = "proxies:\n  - name: a\n    type: ss\n    server: 1.2.3.4\n    port: 443\n    cipher: aes-128-gcm\n    password: x\n";

#[tokio::test]
async fn fetch_ok_with_userinfo_header() {
    let h: MockHandle = spawn_once(200,
        vec![("subscription-userinfo", "upload=1; download=2; total=100; expire=1798732800".into())],
        MIN_YAML.as_bytes().to_vec());
    let url = format!("http://127.0.0.1:{}/sub", h.port);
    // let r = fetch::fetch_once(&url, &cfg()).await.unwrap();
    // assert_eq!(r.userinfo.unwrap().total, Some(100));
    let _ = url;
}

#[tokio::test]
async fn fetch_500_maps_to_http_status() {
    let h = spawn_once(500, vec![], b"err".to_vec());
    let url = format!("http://127.0.0.1:{}/sub", h.port);
    // let e = fetch::fetch_once(&url, &cfg()).await.unwrap_err();
    // assert_eq!(e.code, "E_HTTP_STATUS");
    let _ = url;
}

#[tokio::test]
async fn fetch_html_maps_to_parse_yaml() {
    let h = spawn_once(200, vec![("content-type", "text/html".into())],
                       b"<!doctype html><html></html>".to_vec());
    let url = format!("http://127.0.0.1:{}/sub", h.port);
    // let e = ...; assert_eq!(e.code, "E_PARSE_YAML");
    let _ = url;
}
```

> 上面留注释是因为 `fetch_once` 的真实签名由 P2 阶段的实现决定。**约束**：实现方必须让
> 这三个测试能零改动地取消注释并跑通——这是 P2 卡的验收项之一。

---

## 12.4 契约一致性测试（L3，防止 AI 漂移）

> 这是本套提示词**最重要的自动化防线**：AI 最容易犯的错是发明命令名（如 `refresh_sub`、`get_traffic`）。

### 12.4.1 命令名/事件名/错误码的唯一来源

```rust
// src-tauri/tests/contract_commands.rs
/// 这份清单必须与 C.6 逐字一致；改契约时才允许改它，并同步改前端。
/// 共 30 个（C.6 v1.1）。注意：**没有** `ui_visibility_set`——窗口可见性由 Rust 侧监听
/// Tauri `WindowEvent` 自行判断，前端不得为此增加 IPC（C.6 末尾明确「不做」）。
const IPC_COMMANDS: &[&str] = &[
    "app_bootstrap",
    "settings_get", "settings_save",
    "subscription_list", "subscription_add", "subscription_update_url",
    "subscription_rename", "subscription_set_enabled", "subscription_set_interval",
    "subscription_remove", "subscription_refresh", "subscription_refresh_all",
    "subscription_reveal_url",
    "traffic_summary",
    "speedtest_start", "speedtest_status", "speedtest_cancel",
    "switch_apply_best", "switch_apply_node", "switch_history",
    "core_status", "core_start", "core_stop", "core_restart", "core_install",
    "core_versions_list",
    "sysproxy_set",
    "profile_preview",
    "logs_tail",
    "diagnostics_export",
];

/// 共 10 个（C.7 v1.1）
const EVENT_NAMES: &[&str] = &[
    "core:state", "core:install-progress",
    "sysproxy:state",
    "subscription:updated", "subscription:progress",
    "traffic:tick", "speedtest:progress", "speedtest:done",
    "switch:applied", "notify",
];

/// C.6 末尾的「不做」清单：出现即契约违规
const FORBIDDEN_COMMANDS: &[&str] = &["ui_visibility_set"];

const ERROR_CODES: &[&str] = &[
    "E_NET_DNS", "E_NET_TIMEOUT", "E_NET_TLS", "E_NET_PROXY", "E_HTTP_STATUS",
    "E_PARSE_YAML", "E_PARSE_EMPTY", "E_USERINFO_MISSING",
    "E_CORE_NOT_FOUND", "E_CORE_START_FAILED", "E_CORE_API_UNAUTH", "E_CORE_PORT_EXHAUSTED",
    "E_STORE_IO", "E_STORE_DECRYPT", "E_PERM_ADMIN", "E_UNSUPPORTED", "E_BUSY",
];

#[test]
fn no_unknown_error_codes_in_source() {
    // 扫描 src/ 下所有 .rs，找形如 E_XXX_YYY 的字面量，必须都在白名单内
    let mut bad = Vec::new();
    for entry in walkdir::WalkDir::new("src").into_iter().filter_map(Result::ok) {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("rs") { continue; }
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if token.starts_with("E_") && token.len() > 3 {
                let known = ERROR_CODES.contains(&token);
                if !known { bad.push(format!("{}: {}", entry.path().display(), token)); }
            }
        }
    }
    assert!(bad.is_empty(), "发现契约外错误码：\n{}", bad.join("\n"));
}

#[test]
fn no_forbidden_commands_in_source() {
    // C.6 末尾「不做 ui_visibility_set」：任何 .rs 出现该标识符即违规
    let mut bad = Vec::new();
    for entry in walkdir::WalkDir::new("src").into_iter().filter_map(Result::ok) {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("rs") { continue; }
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for forbid in FORBIDDEN_COMMANDS {
            if text.contains(forbid) {
                bad.push(format!("{}: 出现契约禁止的命令 {forbid}", entry.path().display()));
            }
        }
    }
    assert!(bad.is_empty(), "契约违规：\n{}", bad.join("\n"));
}

#[test]
fn generated_handler_invokes_all_commands() {
    // Tauri 2 的 generate_handler! 里列出的命令必须恰好等于 IPC_COMMANDS（不多不少）
    let src = std::fs::read_to_string("src/lib.rs")
        .or_else(|_| std::fs::read_to_string("src/main.rs"))
        .expect("找不到 src/lib.rs 或 src/main.rs");
    let block = src.split("generate_handler!").nth(1)
        .expect("未找到 generate_handler! 宏，IPC 层未按 C.2 组织");
    let mut registered: Vec<&str> = Vec::new();
    for cmd in IPC_COMMANDS {
        if block.contains(cmd) { registered.push(cmd); }
    }
    assert_eq!(registered.len(), IPC_COMMANDS.len(),
        "generate_handler! 注册的命令数({})与 C.6({})不一致；已注册：{:?}",
        registered.len(), IPC_COMMANDS.len(), registered);
}
```

```ts
// src/ipc/contract.test.ts（vitest 或 node:test 均可）
import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';

const EXPECTED = [
  'app_bootstrap','settings_get','settings_save',
  'subscription_list','subscription_add','subscription_update_url',
  'subscription_rename','subscription_set_enabled','subscription_set_interval',
  'subscription_remove','subscription_refresh','subscription_refresh_all',
  'subscription_reveal_url','traffic_summary',
  'speedtest_start','speedtest_status','speedtest_cancel',
  'switch_apply_best','switch_apply_node','switch_history',
  'core_status','core_start','core_stop','core_restart','core_install','core_versions_list',
  'sysproxy_set','profile_preview','logs_tail','diagnostics_export',
];
// C.6 明确「不做」的命令：出现即违规
const FORBIDDEN = ['ui_visibility_set'];

describe('IPC 契约', () => {
  it('client.ts 中出现的 invoke 名称必须全部在 C.6 白名单内', () => {
    const src = readFileSync('src/ipc/client.ts', 'utf8');
    const found = [...src.matchAll(/invoke(?:<[^>]*>)?\(\s*['"]([a-z_]+)['"]/g)].map(m => m[1]);
    for (const name of found) {
      expect(FORBIDDEN, `C.6 明确禁止的命令 ${name}`).not.toContain(name);
      expect(EXPECTED, `未登记的命令 ${name}`).toContain(name);
    }
  });

  it('必需命令都必须有封装函数', () => {
    const src = readFileSync('src/ipc/client.ts', 'utf8');
    for (const name of EXPECTED) expect(src, `缺少命令 ${name} 的封装`).toContain(name);
  });
});
```

### 12.4.2 `kind` 取值大小写敏感测试（C.6.1 头号陷阱）

> **为什么单独立节**：`GroupInfo.kind` 存的是 mihomo **API 值**（大驼峰 `Selector`），
> 不是订阅 YAML 里写的 `select`（小写、连字符）。写错比较方式不会报错、不会 panic，
> 只会让 `proxies_selectable` 永远返回空 → 「自动切换到最低延迟节点」**静默失效**，
> 用户看到的是「功能没反应」，日志里也没有任何错误。**这是整套提示词里后果最严重的一处静默 bug。**

```rust
// src-tauri/tests/contract_selector_kind.rs
//
// 测试名统一前缀 `kind_case_sensitive`，便于按名字搜索与 CI 单独过滤：
//   cargo test kind_case_sensitive

use clash_zhengl_lib::kernel::api::{GroupInfo, SelectableGroup};

fn group(name: &str, kind: &str) -> GroupInfo {
    GroupInfo {
        name: name.into(),
        kind: kind.into(),          // 原样保存 API 返回的 type 字符串
        now: Some("A".into()),
        all: vec!["A".into(), "B".into()],
        test_url: None,
        hidden: false,
    }
}

/// 1) API 值 "Selector"（大驼峰）必须被选中
#[test]
fn kind_case_sensitive_selector_is_selectable() {
    let g = group("节点选择", "Selector");
    assert!(g.is_selector(), "API 返回 \"Selector\" 时必须判定为可选组");
    let s: SelectableGroup = g.to_selectable().expect("Selector 组必须能转成 SelectableGroup");
    assert_eq!(s.members, vec!["A".to_string(), "B".to_string()]);
}

/// 2) YAML 值 "select"（小写）必须**不**被选中 —— 它是配置写法，不是 API 返回值
#[test]
fn kind_case_sensitive_lowercase_select_is_not_selectable() {
    let g = group("节点选择", "select");
    assert!(!g.is_selector(), "小写 \"select\" 不是 API 值，绝不能被判为可选组");
    assert!(g.to_selectable().is_none(), "小写 \"select\" 必须返回 None");
}

/// 3) 其它组类型（URLTest / Fallback / LoadBalance）一律不选中
#[test]
fn kind_case_sensitive_other_group_kinds_are_not_selectable() {
    for kind in ["URLTest", "Fallback", "LoadBalance", "Relay", "url-test", "fallback"] {
        let g = group("自动选择", kind);
        assert!(!g.is_selector(), "组类型 {kind} 不得被判为可选组");
        assert!(g.to_selectable().is_none(), "组类型 {kind} 必须返回 None");
    }
}

/// 4) 回归保护：禁止用 contains / to_lowercase 之类"宽松比较"
///    "Selector".to_lowercase() == "selector" ≠ "select"，
///    而 "select".contains("select") == true —— 两种写法都会把语义搞错。
#[test]
fn kind_case_sensitive_no_loose_comparison_in_source() {
    let src = std::fs::read_to_string("src-tauri/src/kernel/api.rs")
        .expect("找不到 src-tauri/src/kernel/api.rs");
    for bad in ["contains(\"select\")", "contains(\"Select\")",
                "to_lowercase() == \"select\"", "to_ascii_lowercase() == \"select\""] {
        assert!(
            !src.contains(bad),
            "kernel/api.rs 出现宽松比较 `{bad}`；kind 判定必须是大小写敏感精确匹配 \"Selector\"（C.6.1）"
        );
    }
    assert!(src.contains("\"Selector\""), "kernel/api.rs 必须以字面量 \"Selector\" 判定组类型");
}

/// 5) 端到端：proxies_selectable 产出的组，kind 必须全部是 "Selector"
#[test]
fn kind_case_sensitive_proxies_selectable_only_returns_selector() {
    let all = vec![
        group("节点选择", "Selector"),
        group("自动选择", "URLTest"),
        group("故障转移", "Fallback"),
    ];
    let selectable: Vec<SelectableGroup> =
        all.iter().filter_map(|g| g.to_selectable()).collect();
    assert_eq!(selectable.len(), 1, "5 个组里只有 1 个 Selector，应只产出 1 个可选组");
    assert_eq!(selectable[0].name, "节点选择");
}
```

**前端侧同样要防**：UI 不该拿到 `kind` 做判断（它是内部字段，不经 IPC 暴露）。
若未来因诊断需要展示，必须原样透传字符串，**不得**在前端做 `kind.toLowerCase() === 'select'` 之类的比较。

### 12.4.3 数据结构字段对齐检查

| 检查项 | 方法 |
| --- | --- |
| Rust DTO 与 TS 类型字段一致 | 在 `commands/dto.rs` 上加一个测试，序列化后比对 `snapshot`（`tests/snapshots/dto/*.json`） |
| camelCase 转换生效 | 断言 `serde_json::to_value(&dto)` 的 key 全是 camelCase（无 `_`） |
| 新增字段有默认值 | 反序列化旧版 JSON（含缺失字段）不报错 |
| `schema_version` 升级 | `settings.json` 缺新字段时可读；`migrate.rs` 有对应用例 |
| `SelectableGroup` 字段名 | 必须是 `members`（不是 `candidates`），且**不含** `hidden`（C.6.1） |

```rust
#[test]
fn dto_keys_are_camel_case() {
    let v = serde_json::to_value(SubscriptionDto::sample()).unwrap();
    for k in v.as_object().unwrap().keys() {
        assert!(!k.contains('_'), "DTO 字段 {k} 未转换为 camelCase");
    }
}
```

### 12.4.4 配置取值回归测试（A.6.1 / A.6.3 / A.6.4）

> **为什么需要**：这几个取值一旦被"顺手优化"改回错误版本，**全都不报错**，但会让功能静默失效：
> `find-process-mode: off` → 机场进程规则永久失效；`ipv6: false` → IPv6 站点/节点不可达；
> `dns.enable: true` → 白开一个用不上的 DNS 服务；`url-test` 组 → 内核与本应用抢当前节点。
> 所以必须在 CI 里钉死，靠人 review 是拦不住的。

```rust
// src-tauri/tests/contract_config_values.rs
//
// 测试名统一前缀 config_value_，可单独过滤：cargo test config_value
use serde_yaml::Value;

fn generated_config() -> Value {
    // 用固定 fixture 订阅生成一份 runtime/config.yaml（纯函数，不碰网络）
    let yaml = clash_zhengl_lib::subscription::manager::config_gen_for_test();
    serde_yaml::from_str(&yaml).expect("生成的 config.yaml 必须是合法 YAML")
}

/// 1) find-process-mode 必须是 strict
#[test]
fn config_value_find_process_mode_is_strict() {
    let c = generated_config();
    let v = c.get("find-process-mode")
        .and_then(Value::as_str)
        .expect("config.yaml 必须显式写 find-process-mode");

    assert_ne!(
        v, "off",
        "find-process-mode 写成 \"off\" 会让内核执行 helper.FindProcess = nil，\
         此后机场下发的所有 PROCESS-NAME / PROCESS-PATH 规则**永久失效**，\
         用户只会看到「某个软件连不上」且日志无任何线索（A.6.1）。必须是 \"strict\"。"
    );
    assert_eq!(v, "strict", "find-process-mode 必须为 \"strict\"，当前为 {v:?}");
}

/// 2) ipv6 必须是 true
#[test]
fn config_value_ipv6_is_true() {
    let c = generated_config();
    let v = c.get("ipv6").and_then(Value::as_bool)
        .expect("config.yaml 必须显式写 ipv6");
    assert!(
        v,
        "ipv6 必须为 true：设为 false 会让纯 IPv6 站点与机场下发的 IPv6 节点不可达，\
         换来的那点地址族查询开销不值得（A.6.1）"
    );
}

/// 3) SystemProxy 档 dns.enable 必须是 false
#[test]
fn config_value_dns_disabled_for_system_proxy() {
    let c = generated_config();
    let v = c.get("dns")
        .and_then(|d| d.get("enable"))
        .and_then(Value::as_bool)
        .expect("config.yaml 必须显式写 dns.enable");

    assert!(
        !v,
        "SystemProxy 档 dns.enable 必须为 false：mihomo 在 !Enable 时执行 \
         dns.ReCreateServer(\"\", nil, nil) 并 return，根本不建 DNS 服务；\
         系统代理走 HTTP CONNECT，域名由内核连到目标后解析，本地 53 端口服务用不上（A.6.3）"
    );
}

/// 4) 所有 proxy-groups 的 type 必须是 select，不得有自动组
#[test]
fn config_value_no_auto_proxy_groups() {
    let c = generated_config();
    let groups = c.get("proxy-groups")
        .and_then(Value::as_sequence)
        .expect("config.yaml 必须有 proxy-groups");
    assert!(!groups.is_empty(), "proxy-groups 不得为空");

    for g in groups {
        let name = g.get("name").and_then(Value::as_str).unwrap_or("<未命名>");
        let ty = g.get("type").and_then(Value::as_str)
            .unwrap_or_else(|| panic!("组 {name} 缺少 type 字段"));
        assert!(
            !matches!(ty, "url-test" | "fallback" | "load-balance"),
            "组 {name} 的 type 是 {ty}：禁止生成自动策略组。\
             内核自带健康检查会与本应用的定时测速形成两个决策者，\
             表现为节点来回跳、用户「选了没用」（A.6.4）。只能是 select。"
        );
        assert_eq!(ty, "select", "组 {name} 的 type 必须为 \"select\"，当前为 {ty:?}");
    }
}

/// 5) 生成器出口的 fail-fast 断言确实存在且被调用
#[test]
fn config_value_assert_no_auto_groups_is_wired() {
    let src = std::fs::read_to_string("src-tauri/src/subscription/manager.rs")
        .expect("找不到 subscription/manager.rs");
    assert!(
        src.contains("assert_no_auto_groups"),
        "manager.rs 必须在生成器出口调用 assert_no_auto_groups()（A.6.4），\
         命中自动组即返回 Err，不要静默生成"
    );
}
```

| 断言 | 期望值 | 改错的后果 |
| --- | --- | --- |
| `find-process-mode` | `"strict"` | `"off"` → 机场 `PROCESS-NAME` 规则**永久静默失效**，某软件连不上且查不出原因 |
| `ipv6` | `true` | `false` → 纯 IPv6 站点与 IPv6 节点不可达 |
| `dns.enable` | `false`（SystemProxy 档） | `true` → 白建一个用不上的 DNS 服务，多一份 resolver 对象 |
| `proxy-groups[*].type` | 全部 `select` | 出现 `url-test`/`fallback`/`load-balance` → 内核与本应用抢当前节点，节点来回跳 |

---

## 12.5 必须写的单元测试清单（按文件，共 130+）

| 文件 | 必测点 | 用例数 |
| --- | --- | --- |
| `subscription/userinfo.rs` | 附录 B.7 的 15 行 + 2 行派生 | 17 |
| `subscription/parse.rs` | 空串/非 YAML/无 proxies/proxies 非数组/类型统计/超限节点/未知 type | 10 |
| `subscription/manager.rs` | 去重（按 name / 指纹）、冲突前缀、禁用订阅不合并、排序稳定 | 8 |
| `scheduler/due.rs` | 首次调度、周期到期、多订阅混合、时间回拨、时间前跳、禁用订阅、间隔下限夹紧 | 9 |
| `scheduler/backoff.rs` | 序列 1,2,4,8,16,30(上限)、抖动落在 0..30s、成功后重置 | 5 |
| `speedtest/rank.rs` | 延迟升序、None 排最后、排除关键词、DIRECT/REJECT 排除、tolerance 判定、地区优先、同延迟按名称稳定、**锁定组被过滤** | 10 |
| `speedtest/selector.rs` | min_dwell 未到不切、failure_threshold 未到不降级、目标同名不重复 PUT、组不存在、**`type!=select` 跳过**、**锁定中跳过**、**锁定到期恢复**、**再次手动选择续期** | 10 |
| `kernel/ports.rs` | 端口占用跳过、连续 3 次失败报错、范围边界 | 4 |
| `kernel/api.rs` | URL 编码（中文/空格/&）、状态码映射 401/404/504、delay=0 视为失败、`GroupInfo` 字段映射、`is_selector()` 大小写敏感判定、受保护名过滤 | 11 |
| `kernel/api.rs`（**kind 专项**） | 见 **12.4.2** 独立一组：`"Selector"` 选中 / `"select"` 不选中 / `URLTest`·`Fallback`·`LoadBalance` 不选中 / 禁止宽松比较 / `proxies_selectable` 只产出 Selector | 5 |
| `kernel/fetch_core.rs` | 资产名正则匹配（compatible/v1/v2/v3）、ARCH 分支、SHA256 校验、`.part` 续传偏移、**续传前重算已有内容摘要** | 7 |
| `system/sysproxy.rs` | journal 快照往返、managed_by_us 判定、外力改动探测 | 5 |
| `store/json_store.rs` | 原子写、损坏文件重命名回退、只读目录报 E_STORE_IO | 4 |
| `store/crypto.rs` | 加解密往返、篡改密文失败、空串 | 3 |
| `store/migrate.rs` | v1→v2 补 `compliance_accepted_at`、未知版本拒绝、幂等 | 3 |
| `traffic` 聚合纯函数 | total=0 忽略、全为 0 → None、has_missing、expire 取最早、used 求和 | 6 |
| `domain/policy.rs` | SwitchPolicy 默认值（含 `manual_override_min=60`）、exclude_keywords 命中、`manual_override_min=0` 不暂停 | 4 |
| `util/time.rs` | 单位换算、剩余天数负数、格式化 | 3 |
| **配置生成器**（`config_gen_for_test`） | 见 **12.4.4** 独立一组：find-process-mode / ipv6 / dns.enable / 无自动组 / assert 已接线 | 5 |
| 合计 | — | **130**（加 12.4 契约测试达 133+） |

---

## 12.6 必测异常清单（28 条：20 条基础 + 4 条 v1.1 + 4 条 v1.1/v1.2 补充）

> 每条都必须有：操作步骤 → 期望现象 → 期望错误码 → 是否可恢复。**全部通过才允许标 P8 完成。**

| # | 场景 | 操作 | 期望现象 | 期望错误码 | 可恢复 |
| --- | --- | --- | --- | --- | --- |
| 1 | 断网拉订阅 | 拔网线/关 WiFi 后点「更新」 | 中文提示「无法解析订阅域名…」或超时，应用不卡死 | `E_NET_DNS` / `E_NET_TIMEOUT` | 恢复网络后重试成功 |
| 2 | 订阅返回 500 | 订阅填 `/500` | 显示「订阅服务器返回 HTTP 500」，旧配置仍可用 | `E_HTTP_STATUS` | 改回正常地址即恢复 |
| 3 | 订阅返回 HTML | 订阅填 `/html` | 「订阅内容不是有效的 Clash 配置」 | `E_PARSE_YAML` | 同上 |
| 4 | 订阅返回空 proxies | Mock 返回 `proxies: []` | 「订阅中没有可用节点…」 | `E_PARSE_EMPTY` | 同上 |
| 5 | 无 userinfo 头 | 订阅填 `/nouser` | 订阅成功，流量卡显示「—」，不报错 | 不报错 | — |
| 6 | 畸形 userinfo | 订阅填 `/junk` | 流量显示「—」或部分值，不崩 | 不报错 | — |
| 7 | 8 MB 超大订阅 | 订阅填 `/big` | 明确拒绝并提示配置过大 | `E_PARSE_YAML` | 换正常订阅 |
| 8 | 慢速订阅 | 订阅填 `/slow`，超时设 2000 | 超时提示 | `E_NET_TIMEOUT` | 调大超时后成功 |
| 9 | 断流订阅 | 订阅填 `/broken` | 提示读取失败，不产生半截 yaml | `E_NET_TIMEOUT` | 重试成功 |
| 10 | 配置文件被改坏 | 手工把 `runtime/config.yaml` 改成非法 YAML 后重启 | 自动重建配置或拒绝启动并给出可读原因，不崩 | `E_PARSE_YAML` / `E_CORE_START_FAILED` | 重建成功 |
| 11 | 内核被手动杀 | 任务管理器结束 `mihomo.exe` | `core:state` 变 Failed→自动重启（退避），托盘图标变灰 | `E_CORE_START_FAILED`（若重启失败） | 自动恢复 |
| 12 | 内核端口被占用 | 先 `nc -l 10801` 占住端口再启动 | 自动换端口重试；全失败提示端口耗尽 | `E_CORE_PORT_EXHAUSTED` | 释放端口后恢复 |
| 13 | secret 不一致 | 手工改 `runtime/secret` 后调 API | 「内核 API 鉴权失败」，自动重建 secret 并重启 | `E_CORE_API_UNAUTH` | 自动恢复 |
| 14 | 内核文件缺失 | 删 `runtime/core/<ver>/mihomo.exe` | 「未安装内核，请先下载」 | `E_CORE_NOT_FOUND` | 点下载恢复 |
| 15 | 内核下载失败 | 断网后点下载内核 | 中文提示 + 可重试，不留半截文件 | `E_NET_TIMEOUT` | 重试成功 |
| 16 | 磁盘只读/满 | 把 `%APPDATA%\ClashZhengl` 设只读 | 「配置写入失败，请检查磁盘权限」，不崩 | `E_STORE_IO` | 恢复权限后成功 |
| 17 | 加密数据换机器 | 拷贝 `%APPDATA%` 到另一台机/另一用户 | 「订阅地址无法解密，请重新填写」 | `E_STORE_DECRYPT` | 重填 |
| 18 | 系统代理残留 | 开代理后强杀进程（任务管理器），再启动 | `sysproxy.journal.json` 快照命中 → 恢复直连；`RuntimeState.sysproxy.managed_by_us` 复位为 false | — | 恢复 |
| 19 | 所有节点测速失败 | 把探测 URL 改成不可达地址 | 测速完成但 best 为空，不切换，提示「无可切换节点」 | 不报错 | 改回即恢复 |
| 20 | 切换目标被拒 | Mock 策略组不含该节点 / 目标为 `DIRECT` | 跳过并记 Warn，不崩；`switch:applied` 不发 | 不报错（Warn 日志） | 下次测速恢复 |

### 12.6.1 v1.1 契约新增项的补充用例（4 条，与上表合计 24 条，编号 21–24）

> 这 4 条来自 C.6/C.7/C.3 的 v1.1 裁决，属新增能力，**必须一起测**。

| # | 场景 | 操作 | 期望现象 | 期望错误码 | 可恢复 |
| --- | --- | --- | --- | --- | --- |
| 21 | 内核下载中断续传 | 下载中强杀进程后重启并重新下载 | `runtime/download/*.part` + `.part.json` 存在时从断点续传；完成后清理临时区 | — | 自动续传 |
| 22 | `core:install-progress` 频率 | 下载 20 MB 内核，观察事件 | 阶段依次 `download→verify→extract→done`；合并后 ≤2 条/秒；`percent` 单调不减 | — | — |
| 23 | 系统代理被外力改动 | 开代理后手工把注册表 `ProxyEnable` 改为 0 | 探测到变化 → 发 `sysproxy:state`，UI 开关自动变关，不发错误弹窗 | — | 用户可再次开启 |
| 24 | `schema_version` 1→2 迁移 | 用 v1.0 的 `settings.json`（无 `compliance_accepted_at`）启动 | 可正常读取，字段补 `None` 并升版本；首启重新弹合规提示 | 不报错 | — |

### 12.6.2 v1.1 / v1.2 契约新增项的补充用例（4 条，合计 28 条，编号 25–28）

> 这 4 条来自 C.3 / C.5 v1.1 与 v1.2 裁决，覆盖的是「状态残留」「续传正确性」「手动覆盖」「None 展示」，
> 都是容易做成「看起来能用其实错了」的地方，**必须单独验证**。

| # | 场景 | 操作 | 期望现象 | 期望错误码 | 可恢复 |
| --- | --- | --- | --- | --- | --- |
| 25 | 系统代理崩溃残留（`cleanExit=false`） | 开代理后强杀进程；手工确认注册表 `ProxyEnable=1` 且 `ProxyServer=127.0.0.1:<本应用端口>`；再启动应用 | 启动守卫读 `runtime/sysproxy.journal.json`，发现 `cleanExit=false` 且当前值仍等于我们写的值 → **自动恢复**为快照里的原值，并弹一次通知「已恢复上次异常退出前的系统代理设置」；`RuntimeState.sysproxy.managed_by_us` 复位 false | — | 自动恢复；若原值为空则关闭代理 |
| 26 | 续传后文件完整性 | 下载内核到 ~50% 强杀；再次触发下载，令其续传完成 | 续传前先对已有 `.part` 内容重算 SHA256 并比对 `.part.json` 里记录的已写偏移摘要；**不得**把新数据直接拼到可能损坏的旧尾部。完成后整包 SHA256 必须等于 Release 公布值，`-t` 自检返回 0 | 校验失败 → `E_CORE_START_FAILED`；网络失败 → `E_NET_TIMEOUT` | 删除 `.part` 重下即可 |
| 27 | 手动锁定后自动切换跳过该组 | ① 手动 `switch_apply_node` 选节点 A（该组进入手动覆盖）② 立即触发测速并 `switch_apply_best` | `switch_apply_best` 返回值**不含**被锁定的组（`all` 里没有该组）；该组 `now` 仍为 A；**不产生**该组的 `switch:applied`。若 scope 内只剩该被锁定组 → 整个返回值为 **`None`**（不是 `{primary: null, all: []}`） | 不报错 | 等 `manual_override_min` 分钟或重启后恢复 |
| 28 | `to_delay=None`（Manual 切换） | 手动 `switch_apply_node` 切到一个本轮未测速的节点 | `switch_history` 里该条 `to_delay` 为 `null`；09 章历史列表显示 `—`，**不显示 0ms**；`to_delay` 不得被填成 `0` | 不报错 | — |

> **#28 的判定要点**：`Option<u32>` 序列化后是 `null`（camelCase 下字段名 `toDelay`）。
> 前端必须判 `toDelay == null` 而不是 `toDelay === 0`；写成 `toDelay || '—'` 会把 `0ms` 也变成 `—`，
> 但 `0ms` 在 Best 场景里是**合法的极快值**（A.4.1 里 `delay==0` 只在探测端点语义下视为失败，
> 而 `SwitchRecord.to_delay` 记录的是已确认的切换结果，不能混淆）。用 `??` 或显式 `=== null`。

### 12.6.3 `switch_apply_best` 返回结构断言（v1.3 定稿）

`switch_apply_best` 返回 **`Option<{primary: SwitchRecord, all: SwitchRecord[]}>`**：
- **有组生效** → `Some({primary, all})`，且 **`primary` 与 `all[0]` 恒等**；
- **无任何组被切换** → 整个返回值是 **`None`**，**不存在 `{primary: null, all: []}` 这种形态**。

这是**破坏性变更**（v1.1 及之前返回单条 `SwitchRecord`），前端与测试都必须按新结构断言：

| 场景 | 返回值 | `all` 长度 | 断言 |
| --- | --- | --- | --- |
| `scope = Group("节点选择")` 单组生效 | `Some({primary, all})` | 1 | `all[0] == primary`（`primary` 与 `all[0]` 恒等） |
| `scope = Global`，3 个 Selector 组都切换成功 | `Some({primary, all})` | 3 | `all.len() == 3`；`primary == all[0]` = **首个实际生效的组**（按实际生效顺序，不按 `group_names` 顺序、不按延迟排序） |
| `scope = Global`，其中 1 组被手动锁定 | `Some({primary, all})` | 2 | `all` 不含被锁定组；`primary == all[0]` 且该组确实生效 |
| 所有组都不需要切（当前已是最优） | **`None`** | — | 返回 `None`；`switch:applied` 不发；UI 提示「当前已是最优节点」 |
| 所有组都被锁定 | **`None`** | — | 返回 `None`；**不报错**；日志 Warn 说明原因 |
| `scope` 内没有任何 `Selector` 组 | **`None`** | — | 返回 `None`；Warn 说明「scope 内无可用 Selector 组」 |

> **前端必须防 TypeError**（与 `toDelay` 陷阱对称的两个坑）：
> 1. `result?.all?.length` —— **不能**写 `result.all.length`，因为 `result` 可能是 `null`，直接取值会抛 `TypeError`；
> 2. `result === null` 与 `result.all.length === 0` 是**两种不同情况**（前者=没有组生效，后者=有生效但…实际不会同时出现，
>    因为 `all` 非空才算生效）。判断「是否有切换」只用 `result === null` 或 `result?.all?.length`，
>    不要用 `!result.all.length` 这类会先解引用的写法。

```rust
// src-tauri/tests/contract_switch.rs
#[test]
fn switch_apply_best_returns_option_of_primary_and_all() {
    // 1) 有生效：Some({primary, all})，且 primary 与 all[0] 恒等
    let some = serde_json::json!({
        "primary": {
            "at": 1798732800i64, "group": "节点选择", "fromNode": "A",
            "toNode": "B", "fromDelay": 120, "toDelay": 45u32, "reason": "Best"
        },
        "all": [{
            "at": 1798732800i64, "group": "节点选择", "fromNode": "A",
            "toNode": "B", "fromDelay": 120, "toDelay": 45u32, "reason": "Best"
        }]
    });
    assert!(some.get("primary").is_some(), "有生效时必须含 primary");
    assert_eq!(some["primary"], some["all"][0], "primary 必须与 all[0] 恒等");

    // 2) 无生效：整体是 null（None），不得是 {primary: null, all: []}
    let none: Option<serde_json::Value> = serde_json::from_value(serde_json::Value::Null).unwrap();
    assert!(none.is_none(), "无任何组生效时必须返回 None");
    assert!(
        serde_json::from_value::<Option<serde_json::Value>>(serde_json::json!({
            "primary": null, "all": []
        })).map(|o| o.is_none()).unwrap_or(false) == false,
        "契约不存在 {primary: null, all: []} 形态——实现返回它即为违规"
    );

    // 3) to_delay 必须是 Option：null 合法，不得被填 0
    let rec: SwitchRecord = serde_json::from_value(serde_json::json!({
        "at": 1798732800i64, "group": "g", "fromNode": null, "toNode": "B",
        "fromDelay": null, "toDelay": null, "reason": "Manual"
    })).expect("Manual 记录 toDelay 为 null 必须可反序列化");
    assert_eq!(rec.to_delay, None, "Manual 切换 to_delay 必须为 None，不得用 0 冒充");
}
```

### 12.6.4 自动化部分（可脚本化的 9 条）

```powershell
# scripts/e2e-smoke.ps1 —— 自动化冒烟：覆盖 #1 #2 #3 #5 #8 #9 #12 #14 #19
$ErrorActionPreference = 'Continue'
$mock = $null
try {
    $mock = Start-Process pwsh -ArgumentList @('-NoProfile','-File','.\tests\mock-sub-server.ps1','-Port','18080') -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 2

    # 用 CLI 形式直接戳应用 IPC 不可行（Tauri 无 CLI），因此改为「服务端可达性 + 内核 /version」双检
    $ok = (Invoke-WebRequest 'http://127.0.0.1:18080/ok' -TimeoutSec 5).StatusCode -eq 200
    Write-Host "Mock /ok 可达：$ok"

    $secret = (Get-Content "$env:LOCALAPPDATA\ClashZhengl\runtime\secret" -Raw -ErrorAction SilentlyContinue)
    if ($secret) {
        $h = @{ Authorization = "Bearer $($secret.Trim())" }
        try {
            $v = Invoke-RestMethod 'http://127.0.0.1:9090/version' -Headers $h -TimeoutSec 3
            Write-Host "内核版本：$($v.version) (meta=$($v.meta))"
        } catch { Write-Host "内核 API 不可用（内核可能未运行）：$($_.Exception.Message)" -ForegroundColor Yellow }
    } else {
        Write-Host '未找到 runtime/secret（内核未启动过）' -ForegroundColor Yellow
    }
} finally {
    if ($mock -and -not $mock.HasExited) { Stop-Process -Id $mock.Id -Force }
}
```

> **说明**：Tauri 应用没有 CLI 入口，很多场景只能在 UI 上点。因此这里明确区分：
> **可自动化**（Mock 服务、内核 API、文件系统断言、性能探针、契约测试）与**必须手工**（UI 交互、托盘、安装卸载）。
> 不要假装用脚本测了 UI。

---

## 12.7 分阶段验收勾选表

> 每阶段结束复制对应小节到 PR/交付说明里逐条打勾。**不打勾不许进下一阶段**（01 章 §5.2）。

### P0 脚手架

- [ ] 仓库结构符合 C.2（目录名、文件位置一致）
- [ ] `cargo fmt --check` 无输出
- [ ] `cargo clippy --all-targets -- -D warnings` 无 warning
- [ ] `cargo test` 通过（≥1 个 smoke）
- [ ] `pnpm tsc --noEmit` 通过
- [ ] `cargo tauri build --no-bundle` 成功
- [ ] 冷启动到托盘 ≤ 1.2s（`perf-probe.ps1 -Mode startup`）
- [ ] 单实例生效：二次启动唤起已有窗口，不产生第二个进程
- [ ] 日志文件生成且格式符合 C.9（含 `<time> <LEVEL> [<module>]`）
- [ ] panic hook 生效：故意 panic 有日志、有中文提示

### P1 内核与系统代理

- [ ] 内核可下载/解压/校验，落在 `runtime/core/<version>/`（C.3）
- [ ] 临时区 `runtime/download/*.part` + `.part.json` 续传可用，完成后清理（C.3 v1.1）
- [ ] `-t` 配置自检通过
- [ ] 内核启动后 `GET /version` 返回 `v1.19.33`
- [ ] `external-controller` 为 `127.0.0.1` + 32 hex secret（C.12）
- [ ] `GET /configs` 回读 `mixed-port` 与期望一致
- [ ] 系统代理开启后浏览器可访问外网；关闭后恢复直连
- [ ] `runtime/sysproxy.journal.json` 快照存在且脱敏（不含订阅信息）
- [ ] 强杀进程后重启，系统代理残留被正确恢复（#18）
- [ ] `RuntimeState.sysproxy.managed_by_us` 语义正确（C.5 v1.1）
- [ ] 外部改动注册表 → 发 `sysproxy:state`，UI 自动同步（#23）
- [ ] 杀内核 → 自动重启（#11）；端口占用 → 换端口（#12）
- [ ] `core_*` 五命令幂等（含 `core_versions_list` 离线返回空数组不报错）
- [ ] `core:install-progress` 四阶段齐全、≤2 条/秒、percent 单调（#22）
- [ ] 性能：M1 ≤ 90 MB、M6 ≤ 3s

### P2 订阅拉取与合并

- [ ] 7 个 Mock 场景行为全部符合 12.3.2 表
- [ ] `subscription_*` 全部命令可调用且返回契约结构
- [ ] 多订阅合并后节点名冲突有前缀，无重名
- [ ] 禁用订阅不参与合并与测速
- [ ] 原子写：强杀进程后 `profiles/*.yaml` 与 `settings.json` 无半截文件
- [ ] 损坏文件回退为 `*.corrupt-<ts>` 且 UI 有提示
- [ ] userinfo 17 个单测全绿
- [ ] 合并 500 节点时峰值内存增量 ≤ 50 MB
- [ ] 订阅 URL 落盘为 DPAPI 密文；UI 默认打码；`subscription_reveal_url` 才解密

### P3 调度器

- [ ] 单一 tick 线程，实测 tick 间隔 ≥ 30s（M9）
- [ ] 周期任务启动有 0..30s 抖动
- [ ] 订阅间隔下限 15 分钟生效（`subscription_set_interval`）
- [ ] 指数退避生效且上限正确，成功后重置
- [ ] 系统时间回拨/前跳不导致任务风暴或永久不触发
- [ ] 15 分钟真实观察：自动更新按周期发生，日志可证
- [ ] 空闲 CPU ≤ 1.5%

### P4 测速与自动切换

- [ ] 并发度受 `speedtest_concurrency` 约束（实测不超过）
- [ ] `speedtest:progress` 实测 ≤ 4 条/秒
- [ ] 300+ 节点测速完成，best 正确，结果可解释（每节点有 delay 或 error）
- [ ] `tolerance_ms` / `min_dwell_sec` / `failure_threshold` 三条防抖生效
- [ ] `switch:applied` 负载为单条 `SwitchRecord`（事件负载不变；变化在命令返回值，见下）
- [ ] **`switch_apply_best` 返回 `Option<{primary, all}>`**（v1.3 定稿）：有生效时 `Some({primary, all})`，`scope=Group` 时 `all.len()==1`、`scope=Global` 且 3 组生效时 `all.len()==3`（12.6.3）
- [ ] **`primary` 与 `all[0]` 恒等**，且 = **首个实际生效的组**（按实际生效顺序，不按 `group_names` 顺序、不按延迟排序）
- [ ] **无任何组被切换时整体返回 `None`**，不是 `{primary: null, all: []}`（12.6.3 第 4–6 行场景）
- [ ] 无需切换时返回 `None`、`switch:applied` 不发、UI 提示「当前已是最优节点」
- [ ] 前端用 `result?.all?.length` 取值，不用 `result.all.length`（否则 `None` 时抛 TypeError）
- [ ] **`SwitchRecord.to_delay` 为 `Option<u32>`**：Manual 切换时为 `null`，不得填 `0`（12.6.2 #28）
- [ ] **手动锁定后自动切换跳过该组**：`switch_apply_node` 后立即测速 → `switch_apply_best` 的 `all` 不含该组，不产生该组 `switch:applied`（12.6.2 #27）
- [ ] `SwitchPolicy.manual_override_min` 默认 60；设为 0 时手动选择可被立刻覆盖；到期后该组恢复参与排序
- [ ] `SwitchRuntimeLocks` 为**纯内存态**：不落盘、不进 `RuntimeState`、重启后清空（C.5 v1.2）
- [ ] 目标组 `type != "select"` 时**跳过并记 Warn**，不盲发 PUT（A.2.1）
- [ ] `switch_policy.scope` 指向的组在 `runtime/config.yaml` 里确为 `type: select`，无同名 `url-test`/`fallback`（C.5 v1.2）
- [ ] 切换后 `GET /proxies/{group}` 的 `now` 与切换目标一致
- [ ] 全失败场景（#19）：不切换、有提示、不崩
- [ ] 测速期 CPU ≤ 25% 且 ≤ 5s
- [ ] `speedtest_start` 幂等（重复调用返回同一 runId）
- [ ] `switch_history {limit?}` 返回按时间倒序、默认 50、上限 200；`runtime/switch-history.json` 只留 200 条（C.3 v1.1）
- [ ] 连续切 210 次后 `runtime/switch-history.json` 中 `records.len() == 200`

### P5 UI 与托盘

- [ ] 三页面可切换，空态/加载态/错误态齐全
- [ ] 流量卡：不限量、无限期、已超量、`—` 四种文案正确（C.5 派生规则）
- [ ] 流量页数据来自 `traffic_summary` 的 `TrafficSummary`，`items` 与 `aggregate` 两级都渲染
- [ ] `has_missing=true` 时聚合数字标注「下限」；`stale=true` 时标注「数据可能过期」（C.5 v1.1）
- [ ] 节点数 > 300 时列表可用（虚拟滚动或分页），滚动不卡
- [ ] 事件风暴（连续测速）下界面不闪烁、不重建列表
- [ ] 托盘菜单：切换节点、开关代理、立即更新、立即测速、退出全部可用
- [ ] 托盘/顶栏代理开关初始态取自 `RuntimeState.sysproxy.enabled`，变化来自 `sysproxy:state`
- [ ] 关闭窗口进托盘（`close_to_tray`），托盘双击恢复
- [ ] 窗口可见性由 Rust 侧 `WindowEvent` 判断——**前端不调用 `ui_visibility_set`**（C.6 明确不做）
- [ ] 首启合规提示：`compliance_accepted_at == None` 时必须弹，接受后写时间戳（C.5 v1.1）
- [ ] 深浅色跟随系统；DPI 125%/150% 不糊
- [ ] 性能：M1 ≤ 90 MB、M2 ≤ 140 MB

### P6 轻量化调优

- [ ] C.11 九项指标全部实测达标（贴数字）
- [ ] `docs/perf-baseline.md` 已填写
- [ ] 若有不达标项：已按 10.7 阶梯处置并留档

### P7 打包发布

- [ ] NSIS 安装包产出，名称符合 `ClashZhengl_<version>_x64-setup.exe`（C.1）
- [ ] 安装包 ≤ 12 MB（M7）；安装后磁盘 ≤ 120 MB（M8）
- [ ] 安装/卸载/覆盖升级均正常；卸载询问是否保留 `%APPDATA%\ClashZhengl`（C.12.7）
- [ ] 无 WebView2 的机器上有引导（bootstrapper 或明确提示）
- [ ] About 页含版本、mihomo 版本与 GPL-3.0 声明、源码获取链接（C.12.5）
- [ ] 首启合规提示（不提供任何节点/订阅）

### P8 验收收尾

- [ ] 12.6 的 28 条异常全部通过（20 条基础 + 4 条 v1.1 + 4 条 v1.1/v1.2 补充）
- [ ] 12.2 的 L1–L7 全绿
- [ ] 12.9 发布前检查表全勾
- [ ] 24 小时挂机：自动更新 + 自动测速 + 自动切换持续工作，无内存增长（RSS 前后差 ≤ 10 MB）
- [ ] 脱敏复查：抓 `logs/` 与诊断包全文搜 `token=` `secret` 订阅域名，无明文

---

## 12.8 发布前检查表

### 12.8.1 代码与构建

- [ ] `cargo fmt --check` / `cargo clippy -D warnings` / `cargo test` 全绿
- [ ] `pnpm tsc --noEmit` / `pnpm biome check` 全绿
- [ ] `cargo tauri build` 成功，产物在 `src-tauri/target/release/bundle/nsis/`
- [ ] 版本号三处一致：`tauri.conf.json`、`Cargo.toml`、`package.json`
- [ ] `Cargo.lock` 与 `pnpm-lock.yaml` 已提交
- [ ] 无 `dbg!` / `println!` 残留（生产路径）
- [ ] 无 `unwrap()` / `expect()` 在生产路径（C.13）
- [ ] release profile 按 10.4.1 配置；`panic` 取值有明确结论
- [ ] 二进制体积 ≤ 15 MB、安装包 ≤ 12 MB

### 12.8.2 安全与合规

- [ ] `external-controller` 仅 `127.0.0.1`，secret 为 32 hex 且每次启动随机
- [ ] 无遥测/统计/崩溃上报 SDK（`cargo tree` 核查）
- [ ] 订阅 URL 与 secret 落盘为 DPAPI 密文
- [ ] 日志与诊断包脱敏（正则覆盖 `token|key|secret|password|auth`）
- [ ] `runtime/sysproxy.journal.json` 与 `runtime/switch-history.json` 不含 URL/token/节点密码
- [ ] 内核分发策略已定并在 About 页声明 GPL-3.0 + 源码链接
- [ ] 首启合规提示存在（`compliance_accepted_at` 机制）；未内置任何节点/订阅
- [ ] 默认非管理员运行（`installMode: currentUser`）

### 12.8.3 功能自检（新装机器跑一遍）

- [ ] 全新 Win11 虚拟机安装 → 首启合规提示 → 加 3 条订阅 → 10 秒内显示流量与到期
- [ ] 开系统代理 → 浏览器可访问外网（`curl https://www.gstatic.com/generate_204`）
- [ ] 设 15 分钟订阅周期 + 5 分钟测速周期 → 观察 2 个周期均自动执行
- [ ] 自动切换到最低延迟节点，切换记录可解释（`switch_history` 可查）
- [ ] 关闭窗口进托盘，托盘菜单全部可用
- [ ] 退出后无残留进程（`Get-Process mihomo,clash-zhengl` 为空）
- [ ] 卸载后 `%LOCALAPPDATA%\ClashZhengl` 清理（或按选择保留）

### 12.8.4 文档

- [ ] `docs/perf-baseline.md` 已更新最终数字
- [ ] 已知问题清单（含 `TODO(待确认)` 项）已列入 Release Notes
- [ ] 未验证项明确写出「未验证」，无「应该」类表述
- [ ] 用户可见的「资源占用」宣传数字与实测一致

---

## 12.9 缺陷分级与处置

| 级别 | 定义 | 示例 | 处置 | 是否阻塞发布 |
| --- | --- | --- | --- | --- |
| **P0 阻断** | 崩溃、数据丢失、凭据泄露、无法启动、系统代理无法恢复导致用户断网 | 启动即崩；订阅 URL 明文进日志；关代理失败后系统仍指向已关闭端口 | 立即修，修完回归全量 | **是** |
| **P1 严重** | 核心功能不可用，但有绕过方式 | 内核起不来（手动重启可恢复）；自动切换完全不工作；流量数字全错 | 当版修复 | **是** |
| **P2 一般** | 功能可用但体验明显受损 | 测速进度事件过多导致 UI 卡；订阅更新偶发失败一次后成功；托盘图标不刷新 | 当版或次版修复 | 否（需 Lead 签字放行） |
| **P3 轻微** | 文案、样式、非关键交互 | 单位少一位小数；深色模式下某边框偏亮；tooltip 文案错字 | 排期修 | 否 |
| **P4 建议** | 优化/新增 | 「测速结果按延迟排序」；「记不住窗口位置」 | 进 backlog | 否 |

**判定口诀**：
- 涉及 `E_STORE_*` / 凭据 / 卸载残留 → 直接按 P0 处理（数据类问题不降级）；
- 涉及 C.11 性能预算超标 → 至少 P1（除非已按 10.7 走完阶梯并获得豁免）；
- 涉及契约偏离（发明命令名、字段名不一致）→ **P0**，因为会让后续所有 Agent 会话一起漂移。

### 12.9.1 缺陷记录模板

```markdown
### BUG-0007 订阅更新失败后旧配置被清空

- 级别：P0
- 阶段：P2
- 环境：Win11 23H2 / v0.1.0-rc1
- 复现：订阅填 /500（Mock），点「更新」→ profiles/<uid>.yaml 被写成空文件
- 期望：失败时保留旧 yaml，仅更新 last_update_status=Failed 与 last_error
- 实际：yaml 被截断为 0 字节，下次启动合并不含该订阅节点
- 根因：fetch 失败分支误用了 `File::create`（先截断再判断）
- 修复：改为「先写 tmp，成功后 rename」（C.3 原子写）
- 回归：12.6 #2 通过；新增单测 `failed_fetch_keeps_old_yaml`
```

---

## 12.10 测试环境矩阵

| 环境 | 版本 | 用途 | 必测 |
| --- | --- | --- | --- |
| Win11 23H2 x64（主力机） | 22631+ | 全量功能 | ✅ |
| Win10 22H2 x64 | 19045 | 兼容性（1809+ 目标） | ✅ 一段 |
| Win11 无 WebView2 运行时 | 干净镜像 | 首启引导 | ✅ |
| 4C8G 低配虚拟机 | — | 性能基线（M1–M9） | ✅ |
| 高 DPI（150%） | — | UI 不糊 | ✅ |
| 中文用户名 + 路径含空格 | — | 路径处理 | ✅ |
| 非管理员账户 | — | 默认非提权运行 | ✅ |
| 离线环境 | 断网 | 内核下载失败提示 | ✅ |

> **中文/空格路径**是最容易漏的：内核路径、日志路径、`-f` 参数都可能因为引号问题失败。
> 用一个名为 `C:\Users\测试 用户\` 的账户跑一遍完整流程。

---

## 12.11 测试命令速查

```powershell
# L1 单元
cargo test --lib
cargo test --lib subscription::userinfo       # 只跑某模块
cargo test --lib -- --nocapture               # 看 println

# L2 集成
cargo test --test '*'
cargo test --test fetch_scenarios -- --nocapture

# L3 契约
cargo test --test contract_commands
pnpm vitest run src/ipc/contract.test.ts

# L6 性能
pwsh -File .\scripts\perf-probe.ps1 -Mode all -Minutes 5 -OutJson .\docs\perf\latest.json

# L5 Mock 服务器
pwsh -File .\tests\mock-sub-server.ps1 -Port 18080 -Verbose

# 全门禁（C.13）
pwsh -Command @'
cargo fmt --check
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo test
if ($LASTEXITCODE -ne 0) { exit 1 }
pnpm tsc --noEmit
if ($LASTEXITCODE -ne 0) { exit 1 }
cargo tauri build --no-bundle
exit $LASTEXITCODE
'@

# 脱敏复查（P8 必跑）
$logs = Get-ChildItem "$env:LOCALAPPDATA\ClashZhengl\logs" -File
$hits = $logs | Select-String -Pattern 'token=|secret=|[a-f0-9]{32}|https?://[^\s]*\?[^\s]*=' 
if ($hits) { $hits | Format-List; throw "发现疑似明文凭据" } else { '脱敏检查通过' }
```

---

## 12.12 交付自检（每次阶段交付前）

- [ ] 本阶段对应的 L1 单测数量达标且全绿
- [ ] 本阶段新增的 IPC 命令已在 `contract_commands.rs` 白名单中（不得新增契约外命令）
- [ ] 本阶段新增的错误码若不在 C.8 中，已提 `CONTRACT-CHANGE-REQUEST:` 而不是自己加
- [ ] 12.6 中与本阶段相关的异常条目已手工验证并记录结果
- [ ] 12.7 对应阶段的勾选表已逐条打勾（含截图/数字/日志片段作为凭证）
- [ ] 未验证项已在交付说明中明确列出

---

## 本章自检清单

> 用途：确认**本章自身**（作为文档）与 C 章（现 v1.2）保持同步。
> 与 12.12「交付自检」不同——那是查产品，这是查文档。契约每次升版后跑一遍。

### 一、契约引用一致性

- [ ] 全章出现的 IPC 命令名**逐一**能在 C.6 找到（含 v1.1 的 `core_versions_list` / `switch_history`）
- [ ] 全章出现的错误码**逐一**能在 C.8 找到（17 个，不多不少）
- [ ] 全章出现的事件名**逐一**能在 C.7 找到（10 个，含 v1.1 的 `sysproxy:state` / `core:install-progress`）
- [ ] 全章**不出现** `ui_visibility_set`（C.6 明确「不做」）
- [ ] 12.4.1 的 `IPC_COMMANDS` 数组长度 = 30，`EVENT_NAMES` 长度 = 10，`ERROR_CODES` 长度 = 17
- [ ] 12.4.1 的 `FORBIDDEN_COMMANDS` 含 `ui_visibility_set`，且两处契约测试（Rust / TS）都断言了它
- [ ] 引用的路径（`runtime/download/`、`runtime/sysproxy.journal.json`、`runtime/switch-history.json`）都在 C.3 中
- [ ] 引用的类型名（`SysProxyState`、`TrafficSummary`、`SwitchRuntimeLocks`、`SwitchRecord`）都在 C.5 中

### 二、v1.2 / v1.3 变更已落地

- [ ] `switch_apply_best` 返回 `Option<{primary, all}>` 已在 **12.6.3** 有 6 场景断言表 + Rust 结构测试
- [ ] `SwitchRecord.to_delay: Option<u32>` 已在 12.6.2 #28 与 P4 勾选表中体现（含 `null` vs `0` 的陷阱说明）
- [ ] `SwitchPolicy.manual_override_min` 与手动覆盖语义已在 12.6.2 #27 与 P4 勾选表中体现
- [ ] 「手动锁定后自动切换跳过该组」已作为**可勾选项**写在 12.7 P4 段
- [ ] `switch_policy.scope` 组必须为 YAML 的 `type: select` 已写进 P4 勾选表
- [ ] `SwitchRuntimeLocks` 是内存态、不落盘、不进 `RuntimeState` 已写明
- [ ] `primary == all[0]`、无生效返回 `None`、不存在 `primary: null` 三点均已写明
- [ ] **`kind` 大小写陷阱已在 12.4.2 独立成节**，5 个测试名均以 `kind_case_sensitive` 开头（可用 `cargo test kind_case_sensitive` 过滤）
- [ ] **12.4.2 含「禁止宽松比较」源码扫描断言**（拦 `contains("select")` / `to_lowercase() == "select"`）
- [ ] `SelectableGroup` 字段名为 `members`（非 `candidates`）、不含 `hidden`，已在 12.4.3 列出
- [ ] **配置取值回归测试已在 12.4.4 独立成节**，5 个测试名均以 `config_value_` 开头（`cargo test config_value` 可过滤）
- [ ] **12.4.4 覆盖 `find-process-mode`/`ipv6`/`dns.enable`/`proxy-groups[].type` 四项**，且失败信息写明改错的后果
- [ ] 12.4.4 含 `assert_no_auto_groups` 已接线的源码扫描断言

### 三、数量自洽

- [ ] 异常清单总数 = **28**（12.6 表头、12.7 P8、L5 行三处数字一致）
- [ ] 单测目标 = **130+**（12.2 表、12.5 标题两处一致）
- [ ] 12.2.2 按阶段 × 必测层的表格内容与 12.7 各阶段勾选表不矛盾
- [ ] 12.5 各文件用例数之和 = 表尾合计

### 四、可执行性

- [ ] 12.3.1 的 Mock 服务器脚本是**完整可运行**的 pwsh 7 脚本（无省略号、无伪代码）
- [ ] 12.3.2 的验证命令逐条可直接复制运行
- [ ] 12.11 的命令速查每条都能在 Windows + pwsh 7 下执行（无 bash-only 语法）
- [ ] 所有 PowerShell 片段不含 `<...>` 占位符以外的非法字符（占位符须明显可辨识）

### 五、文档质量

- [ ] 每张表都有表头与分隔行，列数一致
- [ ] 无「应该通过」「理论上没问题」类表述（01 章 §8）
- [ ] 每个验收项都是**可判定**的（有数字、有命令、有明确现象），不是「功能正常」这种空话
- [ ] 缺陷分级（12.9）覆盖了数据类与契约偏离类，且都定为不可让步的级别

---

## 契约对齐记录（v1.4）

| 契约条目 | 本章落点 | 状态 |
| --- | --- | --- |
| v1.0 · 21 条命令 / 8 事件 / 17 错误码 | 12.4.1 白名单 | ✅ 已升级 |
| v1.1 · `core_versions_list` / `switch_history` | 12.4.1 Rust + TS 两处白名单、12.7 P1/P4 | ✅ 已落地 |
| v1.1 · `sysproxy:state` / `core:install-progress` | 12.4.1 `EVENT_NAMES`、12.6 #22/#23、12.7 P1 | ✅ 已落地 |
| v1.1 · `RuntimeState.sysproxy`（`managed_by_us`） | 12.6 #18/#23/#25、12.7 P1/P5 | ✅ 已落地 |
| v1.1 · `TrafficSummary` 聚合规则 | 12.2.1 表、12.5 单测、12.7 P5 | ✅ 已落地 |
| v1.1 · `compliance_accepted_at` / schema v2 | 12.6 #24、12.7 P5 | ✅ 已落地 |
| v1.1 · `runtime/download/` 续传 | 12.6 #21/#26、12.7 P1 | ✅ 已落地 |
| v1.1 · C.11 体积口径 + 100%–120% 仲裁 | 12.7 P6、12.8.1（引用 10 章 10.1） | ✅ 已落地 |
| v1.2 · `SwitchRecord.to_delay: Option<u32>` | 12.6.2 #28 + 12.6.3 Rust 断言 + P4 | ✅ 已落地 |
| v1.2 · `manual_override_min` / 手动覆盖 | 12.6.2 #27 + P4 三连勾选 | ✅ 已落地 |
| v1.2 · scope 组必须 `type: select` | P4 勾选表末段 | ✅ 已落地 |
| **v1.3 · `switch_apply_best` 返回 `Option<{primary, all}>`，`primary == all[0]`，无生效返回 `None`** | **12.6.3 整节重写**（6 场景断言表 + Rust 结构测试）+ P4 勾选表 4 条 | ✅ 已落地 |
| **v1.3 · 不存在 `{primary: null, all: []}` 形态** | 12.6.3 Rust 断言第 2 项 + 12.6.2 #27 + P4 勾选 | ✅ 已落地 |
| **v1.3 · `primary` = 首个实际生效组（按生效顺序，非 group_names 序、非延迟序）** | 12.6.3 表格第 2 行 + P4 勾选 | ✅ 已落地 |
| **v1.3 · §C.6.1 内部接口冻结面**（`SelectorApi`/`ApiError`/`GroupInfo`/`SelectableGroup`/`SwitchRuntimeLocks`，不是 IPC） | 12.2.1 表格（`kernel/api.rs` 行）、12.5 `selector.rs`/`api.rs` 用例 | ✅ 已落地 |
| **v1.3 · 前端 `result?.all?.length`（防 TypeError）** | 12.6.3 末尾「前端必须防 TypeError」+ P4 勾选 | ✅ 已落地 |
| **v1.3 · `kind` 取 API 值 `Selector`（非 YAML 的 `select`）** | **12.4.2 独立小节**（5 个 `kind_case_sensitive_*` 测试，含「禁止宽松比较」的源码扫描断言）+ 12.2.1 两行 + 12.5 专项 5 例 | ✅ 已落地 |
| **v1.4 · 配置取值钉死**：`find-process-mode: strict`（`off` 致 PROCESS 规则永久失效）、`ipv6: true`、`dns.enable: false` | **12.4.4 独立小节**（5 个 `config_value_*` 测试，失败信息均写明改错后果）+ 12.2.1 配置生成器行 + 12.5 专项 5 例 | ✅ 已落地 |
| **v1.4 · §A.6.4 禁止生成自动策略组** | **12.4.4 第 4 项**（`proxy-groups[].type` 全为 `select`）+ 第 5 项（`assert_no_auto_groups` 已接线源码扫描） | ✅ 已落地 |
| **v1.4 · `SelectorApi` 的 `ApiFuture` dyn 兼容形式** | 本章不涉及（实现细节在 07/08 章；13 章 P4 卡已同步） | — |

### 未决项

**无。** 原 3 条未决项已由 C 章 v1.3 全部裁决，v1.4 另补配置取值钉死与 `ApiFuture` 形式：
- `SelectorApi` / `GroupInfo` / `SelectableGroup` 已纳入 §C.6.1 冻结面，本文按「内部接口、不是 IPC、不得新增 invoke」表述；
- `primary` 选取规则已定稿为「`primary == all[0]` = 首个实际生效的组」；
- 无生效时整体返回 `None`，契约明确「不存在 `primary: null` 形态」。
