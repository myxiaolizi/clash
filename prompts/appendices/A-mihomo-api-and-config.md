# 附录 A · mihomo REST API 与配置字段速查

> **适用版本**：mihomo（Clash.Meta）**v1.19.33**（本套提示词锁定的内核版本）。
> **权威边界**：本附录只描述「本项目实际会调用的端点与字段」。任何未在此列出的端点，
> 默认不接入产品代码；确需接入时按 `CONTRACT-CHANGE-REQUEST:` 提交流程走。
> **契约依赖**：[C-conventions.md](C-conventions.md) §C.3（路径）、§C.6（IPC）、§C.8（错误码）、§C.12（安全）。
> **不自造**：字段名一律以下表与真实响应为准，禁止凭记忆编造参数名。

---

## A.1 连接基线（写代码前先固化这 6 条）

| 项 | 取值 | 依据 |
| --- | --- | --- |
| 监听地址 | `127.0.0.1:<port>`（`bind-address: 127.0.0.1`） | C.12 第 2 条，禁止 `0.0.0.0` |
| 端口 | `0` = 自动分配（先 `TcpListener::bind("127.0.0.1:0")` 探一个空闲端口再释放） | C.5 `api_port` |
| secret | 每次启动随机 32 位 hex，写入 `runtime/secret`（ACL 仅当前用户） | C.3 / C.12 |
| 鉴权头 | `Authorization: Bearer <secret>`，**除 `/` 与 `/ui` 外全部路径** | 本文 A.3 |
| 超时 | 连接 1s / 读 3s（健康检查可放宽到 5s），禁止无超时 `await` | C.13 |
| 客户端 | `reqwest` 复用单个 `Client`，`no_proxy()`（**关键**：不要走系统代理/环境变量代理，否则回环被代理走） | 本文 A.9 坑 5 |

> **回环不走代理**：Windows 上若用户开了系统代理，`reqwest` 默认读 `HTTP_PROXY` 环境变量；
> 必须 `Client::builder().no_proxy()`，否则本地 API 调用会被自己代理掉，表现为「一直 502/超时」。

---

## A.2 端点速查表

方法 / 路径 / 关键参数 / 成功返回 / 本项目用途 / 备注。全部路径前缀为 `http://127.0.0.1:<port>`。

| 方法 | 路径 | 关键参数 | 成功返回 | 本项目用途 | 备注 |
| --- | --- | --- | --- | --- | --- |
| GET | `/version` | — | `{"meta":true,"version":"v1.19.33"}` | 健康检查、内核版本核对 | 最轻的一次探活；`meta:true` 表示是 mihomo 系 |
| GET | `/configs` | — | 大对象：`port/socks-port/mixed-port/allow-lan/mode/log-level/ipv6/tun…` | 启动后核对 `mixed-port` 是否是我们要的端口 | 只读，不落盘 |
| PUT | `/configs` | body `{"path":"<abs yaml>"}` | `204` | 配置重建后热重载 | 会重读文件；失败可能直接导致内核退出，慎用 |
| PATCH | `/configs` | body 局部字段，如 `{"mode":"rule"}` / `{"log-level":"info"}` | `204` | 切模式、调日志级别 | 只传要改的字段；不支持改 `secret` |
| GET | `/proxies` | — | `{"proxies":{"<name>":{"type":"…","now":"…","all":[…],"history":[{"time":…,"delay":…}],"udp":true,"alive":true}}}` | 节点/组枚举、`now` 读当前选中 | **不返回节点密码**，可安全落日志（需脱敏名） |
| GET | `/proxies/{name}` | 名称需 URL 编码 | 单个节点对象（同上） | 单节点详情/当前选中 | 名称含空格、中文、`&` 必须 `encode` |
| PUT | `/proxies/{name}` | body `{"name":"Japan"}` | `204` | **切换 selector 组**（本项目唯一写入口） | `{name}` 是**组名**；body 是目标节点名。**只能用于 `type: select` 组**，见 A.2.1 |
| DELETE | `/proxies/{name}` | — | `204` | **不使用**（会删运行态节点，破坏一致性） | 列入禁用清单，理由见 A.2.2 |
| GET | `/proxies/{name}/delay` | `url`、`timeout`（ms）、`expected`（期望状态码，默认 204） | `{"delay":123}` | **单节点测速**（P4 核心） | 失败非 200；见 A.4 |
| GET | `/group` | — | `{"proxies":[…]}` | 列出所有策略组（含 `select`/`url-test`） | 组信息比 `/proxies` 更聚焦 |
| GET | `/group/{name}/delay` | `url`、`timeout` | `{"节点名":123,"节点名2":98,…}` | 整组并发测速（备选方案） | **副作用**：会清空该自动策略组的固定选择，见 A.4 |
| GET | `/traffic` | 无（GET 拉一条 / WS 流式） | `{"up":1024,"down":2048,"upTotal":…,"downTotal":…}` | 速率与累计（`traffic:tick` 数据源） | 单位：`up/down` 字节每秒，`*Total` 字节累计；约每秒一条 |
| GET | `/connections` | — | `{"downloadTotal":…,"uploadTotal":…,"connections":[…]}` | 诊断页（可选） | 返回可能很大，**禁止周期轮询**，仅诊断导出时拉一次 |
| DELETE | `/connections` | — | `204` | 不使用（断全部连接，用户会感知断网） | 列入禁用清单 |
| GET | `/providers/proxies` | — | `{"providers":{"<name>":{"type":"Proxy","vehicleType":"HTTP","proxies":[…]}}}` | 排查「订阅节点没进内核」 | 我们的配置用内联 `proxies`，正常应为空或仅默认 provider |
| PUT | `/providers/proxies/{name}` | body `{"path":…}` | `204` | 不使用（我们走 `/configs`） | 仅在改用 provider 方案时才会用到 |
| GET | `/providers/proxies/{name}/healthcheck` | — | `204` | 不使用（与自带测速冲突） | 触发 provider 内部测速，会与 P4 打架 |
| GET | `/rules` | — | `{"rules":[{"type":"DOMAIN-SUFFIX","payload":"…","proxy":"…"}]}` | 诊断导出、规则命中排查 | 大订阅可达数千条，**不常驻内存**，按需取 |
| GET | `/logs` | WS，`?level=info` | WS 帧 `{"type":"info","payload":"…"}` | 不使用（我们有内核 stdout 文件） | 若接入须脱敏，且与 C.9「内核 stdout 单独文件」冲突 |
| GET | `/memory` | — | 部分构建返回 `{"inuse":…,"oslimit":…}` | 不使用 | 字段随构建变化，不可依赖 |

**禁用端点清单（硬约束）**：`DELETE /proxies/{name}`、`DELETE /connections`、`PUT /providers/proxies/{name}`、
`GET /providers/proxies/{name}/healthcheck`、任何 `/restart`、`/upgrade` 类端点。
理由：会破坏我们持有的运行态副本，或与定时任务互相踩踏。

### A.2.1 `PUT /proxies/{name}` 只能用于 Selector 组（v1.3 强调）

| YAML 配置 | `/proxies` 的 `type` | PUT 行为 | 内核是否接受 | 本项目是否使用 |
| --- | --- | --- | --- | --- |
| `select` | **`Selector`** | 立即生效，`now` 变为目标节点 | 接受，`204` | ✅ **唯一允许的写入目标** |
| `url-test` | `URLTest` | 内核接受并改 `now`，但**自带健康检查会在下个 `interval` 改回去** | 接受，但行为不可靠 | ❌ 禁止 |
| `fallback` | `Fallback` | 同上，且失败切换逻辑会覆盖我们的选择 | 接受，但行为不可靠 | ❌ 禁止 |
| `load-balance` | `LoadBalance` | 无「当前选中」概念，PUT 无实际意义 | 接受但无效果 | ❌ 禁止 |
| `relay` | `Relay` | 无选中语义 | 接受但无效果 | ❌ 禁止 |
| — | 单节点（如 `HongKong-01`） | 返回 `400`（受保护名除外） | 拒绝 | ❌ 禁止 |

**结论与落地约束**：
1. 合并配置时，`switch_policy.scope` 指向的组**必须**生成为 YAML 的 `type: select`（C.5「组类型约束」）。
2. `selector.rs` 在 PUT 之前**必须**先 `GET /proxies/{group}` 校验 **`type == "Selector"`**（API 返回的是大驼峰，
   不是 YAML 里的小写 `select`）；不是 `Selector` 则跳过并记 Warn，**不得**盲发 PUT
   （否则会出现「API 说切了，内核又切回去」的假成功）。
3. 判定用 `GET /proxies/{group}` 返回体里的 `type` 字段，不要靠组名猜，也不要用 `"select"` 小写比较
   （写错会表现为「所有组被跳过、自动切换完全不工作」）。

### A.2.2 `DELETE /proxies/{name}` 为什么禁用

| 风险 | 说明 |
| --- | --- |
| 语义是「删除运行态代理/组」 | 不是「取消选中」。删掉后该节点从 `/proxies` 消失，但我们持有的 `selected`/`ProbeResult` 仍指向它 |
| 不可逆 | 内核不会因为下次 `/configs` PUT 就恢复你删掉的东西（要重启才重读配置） |
| 会连带破坏规则引用 | `rules` 里引用该节点/组会变成悬空引用，流量走向不可预期 |
| 与测速并发冲突 | 测速正在对某节点发 delay 请求时被删，返回 404 被记成「节点不可用」，污染本轮结果 |

**替代方案**：需要「停用某个节点」时，改配置（从 `proxies` 数组移除）后重建 `runtime/config.yaml`
并重启内核或 `PUT /configs`，**不要**用 `DELETE /proxies`。

---

### A.2.3 `/proxies` 响应字段映射（`GroupInfo` / `SelectableGroup`）

`/proxies`（以及 `/proxies/{name}`）的返回体里，每个条目都是一个「proxies 对象」。
策略组是其中 `all` 非空的一种特殊条目。本项目**只取以下 7 个字段**，其余一律忽略：

| 字段 | 类型 | 出现位置 | 本项目用途 | 备注 |
| --- | --- | --- | --- | --- |
| `name` | string | 节点与组都有 | 作为 `node_key` / `group` 的真源 | 已是内核里的唯一名；冲突前缀在合并阶段已加好 |
| `type` | string | 节点与组都有 | **判定是否可选**（见下方「kind 取值」） | mihomo 返回值是 **`Selector`**（大驼峰），不是 YAML 里写的 `select`；两者是「API 值 vs 配置值」的区别 |
| `now` | string | **仅组有** | 读当前选中节点（`SelectedNode.node`） | 单节点条目无此字段 |
| `all` | string[] | **仅组有** | 组内候选列表；空/缺失 = 不是组 | 数组顺序即配置里的顺序，**不保证按延迟排** |
| `history` | object[] | 节点与组都可能 | 取最后一条的 `delay` 做「内核对延迟的旁证」 | 元素形如 `{"time":"2026-11-06T16:00:00+08:00","delay":123}`；本项目**以自己测速结果为准**，此字段仅诊断用 |
| `testUrl` | string? | `url-test` 组常有 | 诊断：核对内核探测 URL 是否与 `settings.speedtest_probe_url` 一致 | 不一致说明两套延迟口径不可比 |
| `hidden` | bool? | 少见 | 诊断：`true` 时该组不该在 UI 列表里展示 | 缺失按 `false` 处理 |

**`kind` / `type` 取值对照（关键，容易写错）**

| YAML 配置写 | `/proxies` 返回的 `type` | 可 PUT | 本项目用途 |
| --- | --- | --- | --- |
| `select` | **`Selector`** | ✅ | 唯一允许的自动/手动切换目标 |
| `url-test` | `URLTest` | ❌ 不可靠（A.2.1） | 只在诊断时识别并跳过 |
| `fallback` | `Fallback` | ❌ 不可靠 | 同上 |
| `load-balance` | `LoadBalance` | ❌ 无意义 | 同上 |
| `relay` | `Relay` | ❌ | 同上 |

> **必须按 `type == "Selector"` 判定**，不要按 YAML 里的 `select` 判，也不要用 `type == "select"` 的小写比较——
> 那会让 `is_select()` 永远返回 false，表现为「所有组都被跳过、自动切换完全不工作」。

**映射到本项目的类型**（这 5 个类型已纳入 **C.6.1 内部接口冻结面**，见 §C.6.1；它们是 `kernel/api.rs` 的内部类型，
**不是 IPC 命令，不得为它们新增 invoke**）：

```rust
// src-tauri/src/kernel/api.rs —— 由 SelectorApi trait 暴露（C.6.1 冻结面）
/// 内核返回的「组」视图：只保留本项目需要的字段。字段名与 C.6.1 一致。
pub struct GroupInfo {
    pub name: String,              // 来自 name
    pub kind: String,              // 来自 type，原样字符串；Selector | URLTest | Fallback | LoadBalance | Relay
    pub now: Option<String>,       // 来自 now；单节点条目为 None
    pub all: Vec<String>,          // 来自 all；空 = 不是组
    pub test_url: Option<String>,  // 来自 testUrl
    pub hidden: bool,              // 来自 hidden，缺失 = false
}

/// 可作为自动切换目标的组：仅 kind == "Selector"（C.6.1 冻结面）
pub struct SelectableGroup {
    pub name: String,
    pub members: Vec<String>,      // 字段名是 members（C.6.1 定稿），来自 all 去掉受保护名后的结果
    pub now: Option<String>,
}

impl GroupInfo {
    /// 只有 Selector 组允许 PUT（A.2.1）。注意比较的是 API 值 "Selector"。
    pub fn is_selector(&self) -> bool { self.kind == "Selector" }
    pub fn to_selectable(&self) -> Option<SelectableGroup> {
        if !self.is_selector() || self.hidden { return None; }
        Some(SelectableGroup {
            name: self.name.clone(),
            members: self.all.iter().filter(|n| !is_protected_name(n)).cloned().collect(),
            now: self.now.clone(),
        })
    }
}

/// C.5 v1.2 的受保护名（PUT 会被 400 拒绝）
fn is_protected_name(n: &str) -> bool {
    matches!(n.to_ascii_uppercase().as_str(),
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE" | "GLOBAL")
}
```

**取数策略**：

| 场景 | 调用 | 频率 |
| --- | --- | --- |
| 启动后建立组视图 | `GET /proxies` 一次，筛出 `all` 非空的条目 | 每次内核启动 / 配置重建后 1 次 |
| 测速前确认组仍存在且是 Selector | `GET /proxies/{group}`，判 `type == "Selector"` | 每轮测速 1 次（不逐节点查） |
| 切换后核对生效 | `GET /proxies/{group}` 读 `now` | 每次 PUT 后 1 次 |
| 诊断导出 | `GET /proxies` 全量 | 仅用户点「导出诊断」时 |

> **禁止周期轮询 `/proxies`**：大订阅下该响应可达数百 KB 到数 MB。运行态靠事件与本地缓存维持，
> 只在上述 4 个时机取。这条与 10 章「禁止轮询 `/connections`、`/rules`」是同一类约束。
>
> **这 5 个类型已纳入 C.6.1 内部接口冻结面**（`SelectorApi` / `ApiError` / `GroupInfo` / `SelectableGroup` /
> `SwitchRuntimeLocks`）：它们定义在 `kernel/api.rs`，**不是 IPC 命令、不得为它们新增 invoke**，
> 但改动须同步 07 章 §7.16 的冻结表并升冻结版本号。

---

## A.3 鉴权与最小调用（pwsh 7 可直接跑）

```powershell
# 假设已从 runtime/secret 读出 secret（仅本机、当前用户可读）
$secret  = (Get-Content "$env:LOCALAPPDATA\ClashZhengl\runtime\secret" -Raw).Trim()
$base    = "http://127.0.0.1:9090"
$headers = @{ Authorization = "Bearer $secret" }

# 1) 健康检查 + 版本
Invoke-RestMethod -Uri "$base/version" -Headers $headers -TimeoutSec 3 |
    Select-Object meta, version

# 2) 单节点延迟（注意 URL 编码：中文/空格必须转义）
$node = [uri]::EscapeDataString("香港 01")
$u    = "$base/proxies/$node/delay?url=$([uri]::EscapeDataString('http://www.gstatic.com/generate_204'))&timeout=5000&expected=204"
Invoke-RestMethod -Uri $u -Headers $headers -TimeoutSec 8

# 3) 读当前 selector 选中
(Invoke-RestMethod -Uri "$base/proxies/$([uri]::EscapeDataString('节点选择'))" -Headers $headers).now

# 4) 切换 selector（PUT + JSON body）
$body = @{ name = "Japan-Tokyo-01" } | ConvertTo-Json -Compress
Invoke-RestMethod -Uri "$base/proxies/$([uri]::EscapeDataString('节点选择'))" `
    -Method Put -Headers $headers -ContentType "application/json" -Body $body
# 成功无响应体（204），Invoke-RestMethod 返回 $null —— 这是正常的
```

> **判定成功别只看返回值**：`204` 时 `Invoke-RestMethod` 返回 `$null`。
> 用 `-SkipHttpErrorCheck`（pwsh 7）拿状态码再判断，或在 Rust 侧判断 `status.is_success()`。

---

## A.4 延迟端点语义（P4 是最容易踩错的地方）

### A.4.1 单节点 `/proxies/{name}/delay`

| 情形 | HTTP | 响应体 | 我们怎么处理 |
| --- | --- | --- | --- |
| 成功 | `200` | `{"delay":123}` | `ProbeResult.delay_ms = Some(123)` |
| 超时 | `504`（部分构建 `500`） | `{"message":"…"}` | `delay_ms = None`，`error = "timeout"` |
| 目标返回非 `expected` | `200` 但 `delay` 可能为 `0`，或非 2xx | — | `delay <= 0` 一律视为失败 |
| 节点名不存在 | `404` | `{"message":"Resource not found"}` | 记 Warn，跳过该节点 |
| 资源名受保护 | `400` | `{"message":"…"}` | 记 Warn，跳过 |

**实现铁律**：
1. `delay == 0` 视为失败（mihomo 在探测失败/未测时可能给 0）。
2. `timeout` 仍要配套 HTTP 客户端超时，取 `timeout_ms + 1500`，避免内核卡死拖住整个测速。
3. 每个探测独立 `reqwest` 请求，受 `Semaphore(speedtest_concurrency)` 约束（C.5 默认 8，上限 16）。
4. 结果**不做二次四舍五入**，`u32` 原样存；`< 1ms` 记 `0`（C.10）。

### A.4.2 整组 `/group/{name}/delay`

```text
GET /group/自动选择/delay?url=http%3A%2F%2Fwww.gstatic.com%2Fgenerate_204&timeout=3000
→ 200 {"香港 01":56,"日本 02":98,"美国 03":210,"新加坡 04":0}
```

| 优点 | 缺点 |
| --- | --- |
| 一次请求测完整组，IPC 与 HTTP 开销最小 | **会清空该自动策略组（`url-test`/`fallback`）已固定的选择**，导致内核自己又切一次 |
| 内核侧并发，我们无需管信号量 | 无法对单个节点做「失败重试一次」，也拿不到失败原因 |
| 返回体直接是「名称 → 延迟」映射 | 组内节点数多时单次响应变大，可能超过我们 3s 读超时 |

**结论（本项目采用）**：
- **主路径用单节点 `/proxies/{name}/delay`**：可控、可重试、可解释、无副作用。
- `/group/{name}/delay` **只在诊断导出/排障命令里手动用**，产品代码不调用。

> 若未来要改用它，必须先在 P4 卡里补一条验收：测速后 `GET /proxies/{group}` 的 `now` 不得被内核改变，
> 否则会出现「刚测完→内核自己切→我们再切」的双切抖动。

---

## A.5 `PATCH /configs` 可改字段（本项目只允许这 4 个）

| 字段 | 取值 | 用途 | 风险 |
| --- | --- | --- | --- |
| `mode` | `"rule"` / `"global"` / `"direct"` | 切模式 | `global` 会让所有流量走同一节点，改前后都要写 `SwitchRecord`/日志 |
| `log-level` | `"silent"` `"error"` `"warning"` `"info"` `"debug"` | 用户在设置里调级别 | 注意内核取值是 `warning` 不是 `warn` |
| `allow-lan` | `false` | 强制关闭 | 只允许写成 `false` |
| `ipv6` | `true` / `false` | 跟随设置 | 需重建 DNS 时用 `/configs` PUT 而非 PATCH |

**禁止 PATCH 的字段**：`port`、`socks-port`、`mixed-port`、`secret`、`external-controller`、`tun`、`dns`。
这些字段变更一律走「重建 `runtime/config.yaml` → 重启内核」，否则会出现「API 说改了、实际监听没变」的假状态。

---

## A.6 配置文件字段表（`runtime/config.yaml`）

> 这张表是 P1/P2 卡里写配置生成器的**唯一依据**。生成器输出的最小骨架见 A.6.2。

### A.6.1 字段清单

| 字段 | 级别 | 取值 | 说明 |
| --- | --- | --- | --- |
| `mixed-port` | **必需** | `1024..65535`（先探测空闲） | 系统代理指向它；HTTP+SOCKS 合一 |
| `bind-address` | **必需** | `"127.0.0.1"` | 与 C.12 一致，禁止 `*`/`0.0.0.0` |
| `external-controller` | **必需** | `"127.0.0.1:<port>"` | 与 `api_port` 一致 |
| `secret` | **必需** | 32 hex 随机串 | 每次启动重新生成 |
| `mode` | **必需** | `"rule"`（默认） | 不用 `global` 做默认 |
| `log-level` | **必需** | `"info"`（默认）/`"warning"`/`"error"`/`"debug"` | 与 `settings.log_level` 映射 |
| `allow-lan` | **必需** | `false` | 轻量客户端不需要局域网共享 |
| `ipv6` | 建议 | `true` | 与系统代理无关（系统代理走 HTTP CONNECT，不是 IP 栈开关）。关掉会让纯 IPv6 站点不可达，且机场若下发 IPv6 节点会连不上。真实收益仅为省一次 `ipv6` 地址族查询，不值得牺牲可达性 |
| `unified-delay` | 建议 | `true` | 延迟口径统一（去掉握手抖动），与我们的测速数值一致 |
| `tcp-concurrent` | 建议 | `true` | 多 IP 并发握手，弱网更稳；对 CPU 影响可忽略 |
| `find-process-mode` | **必需** | `"strict"` | **禁止写 `"off"`**。mihomo `tunnel/tunnel.go` 对 `FindProcessOff` 执行 `helper.FindProcess = nil`，此后所有 `PROCESS-NAME`/`PROCESS-PATH` 规则**永久静默失效**（机场配置里极常见），用户表现为「某个软件连不上」却查不出原因。`strict` 是内核默认值，本项目保持默认 |
| `profile.store-selected` | **必需** | `true` | 让手动选择持久化，重启不丢 |
| `profile.store-fake-ip` | 建议 | `false` | SystemProxy 档 `dns.enable: false` 时 fake-ip 根本不启用，此键无作用；写成 `false` 是为了不让人误以为我们在用 fake-ip。将来开 TUN 时再改 `true` |
| `geodata-mode` | 建议 | `false` | 用 `geosite`/`mmdb` 文件而非内嵌，减小常驻内存 |
| `geo-auto-update` | **必需** | `false` | 禁止内核偷偷联网下载 geo 数据（违反「不外传/只访问用户配置地址」） |
| `external-ui` | **禁用** | 不出现该键 | 一旦设置会起面板静态服务，增大暴露面与内存 |
| `external-controller-tls` | **禁用** | 不出现 | 回环不需要 TLS，且会多读证书文件 |
| `authentication` | **禁用** | 不出现 | 与 Bearer secret 重叠，徒增复杂度 |
| `allow-lan` 为真 | **禁用** | — | 见上 |
| `tun` | **禁用在 MVP** | 不出现或 `enable: false` | TUN 需管理员 + wintun，属 `E_PERM_ADMIN` 分支 |
| `redir-port`/`tproxy-port`/`interface-name`/`routing-mark` | **禁用** | 不出现 | Linux/macOS 专用，Windows 上无意义 |
| `socks-port`/`port` | 可选 | 不出现 | 有 `mixed-port` 就不需要分开写 |
| `dns` | 建议 | 见 A.6.3 | SystemProxy 档**关闭**（`enable: false`）；不配会退回系统 DNS |
| `proxies` | **必需** | 合并后的节点数组 | 见 04 章 |
| `proxy-groups` | **必需** | 至少一个 `select` 组 | 见 04 章 |
| `rules` | **必需** | 至少 `MATCH,<组名>` 兜底 | 空 `rules` 会导致所有流量直连 |
| `providers` | 不使用 | 不出现 | 我们不用 provider，避免二次拉取订阅 |

### A.6.2 最小骨架（生成器输出模板）

```yaml
# runtime/config.yaml —— 由 merge 阶段生成，禁止人工编辑
mixed-port: 10801
external-controller: 127.0.0.1:49152
secret: "3f9c0a7e5b2d4c8f1a6e9b0d3c7f5a21"
mode: rule
log-level: info
allow-lan: false
ipv6: true
unified-delay: true
tcp-concurrent: true
find-process-mode: strict
profile:
  store-selected: true
  store-fake-ip: false
geodata-mode: false
geo-auto-update: false
dns:
  enable: false          # SystemProxy 档：CONNECT 域名由内核代解析，本地 DNS 服务无用
  ipv6: true
proxies: []          # 04 章合并结果
proxy-groups:
  - name: Proxy
    type: select
    proxies: []      # 合并后的节点名 + DIRECT
rules:
  - GEOIP,LAN,DIRECT,no-resolve
  - MATCH,Proxy
```

> **为什么 `bind-address` 不出现**：只在 `allow-lan: true` 时才有意义。我们固定 `allow-lan: false`，
> 写它会造成"配了但没生效"的误解。
>
> **为什么 `proxy-groups` 只有一个 `select` 组**：见下方 A.6.4。这是 C.5 契约的硬约束。

### A.6.3 DNS / 分组取值注意

| 项 | 注意点 |
| --- | --- |
| `dns.enable` 的取值 | **SystemProxy 档必须 `false`**。mihomo `hub/executor/executor.go::updateDNS` 在 `!Enable` 时直接 `dns.ReCreateServer("", nil, nil)` 并返回——**完全不创建 DNS 服务、不监听任何端口**。系统代理模式下流量是 HTTP CONNECT，域名由内核连到目标后再解析，本地 53 端口服务根本用不上。设 `true` 的净效果只是多开一个服务 + 多一份 resolver 对象，纯亏 |
| `dns.enable` 何时该开 | 只有将来做 **TUN 模式**时才需要（TUN 会让 DNS 请求真的进到内核）。届时再改成 `true` + `enhanced-mode: fake-ip`，属 P6 之后的可选阶段 |
| `dns.nameserver` | 若开启，只写**不需要代理就能解析**的公共 DNS；写机场内网 DNS 会在启动初期解析失败 |
| `ipv6` 与 `dns.ipv6` | 两个开关都要 `true`。`ipv6: false` 会让纯 IPv6 站点与 IPv6 节点不可达；`dns.ipv6` 只在 `dns.enable=true` 时生效 |
| **禁止生成 `url-test` / `fallback` 组** | 见 A.6.4，这是会直接破坏自动切换的硬约束 |

### A.6.4 ⚠️ 禁止生成自动策略组（与 C.5 契约一致）

**规则：`runtime/config.yaml` 里所有 `proxy-groups` 必须是 `type: select`，不得出现 `url-test` / `fallback` / `load-balance`。**

原因：内核自带健康检查与我们的定时测速会形成**两个决策者**，表现为节点来回跳。

| 错误写法 | 会发生什么 |
| --- | --- |
| `select` 组里嵌套一个 `url-test` 子组 | 用户在 `select` 里选了某节点，`url-test` 按自己的 `interval` 又改回去 → 用户感觉"选了没用" |
| 同名 `select` + `url-test` 并存 | 我们的 `PUT /proxies/{select组}` 改了选择，`url-test` 下一个 `interval` 又切走 → 切换日志显示切了但延迟没变 |
| `fallback` 组 | 该组按可用性自动降级，覆盖用户选择，且它探测失败与我们 `failure_threshold` 判据不同 → 两边各降一次 |

正确做法：只有 `select` 组，自动切换**完全由本应用**通过
`PUT /proxies/{group} {"name":"<节点>"}` 显式完成（见 07 章）。

> **`url-test` 的唯一合法用途**：仅当用户手动在设置里显式开启"交给内核自选（省 CPU）"时，
> 才允许生成一个 `url-test` 组，且此时必须**同时关闭**本应用的自动切换（`switch_policy.enabled = false`），
> 二者绝不可同时生效。该选项在 MVP 中**不提供**。
>
> 04 章已用 `assert_no_auto_groups()` 在生成器出口做断言，命中即 `Err`（fail fast）。

---

## A.7 Windows Release 资产名规则与选择策略

v1.19.33 的 Windows 资产（同名规则适用其它版本）：

| 资产名 | CPU 指令集 | 兼容性 | 体积量级 | 说明 |
| --- | --- | --- | --- | --- |
| `mihomo-windows-amd64-compatible-v1.19.33.zip` | GOAMD64=v1 | **最高**（老 CPU 也能跑） | 略大 | **本项目默认首选** |
| `mihomo-windows-amd64-v1.19.33.zip` | 默认构建 | 高 | 中 | 语义随构建脚本变化，**不要假设**等于 v1 |
| `mihomo-windows-amd64-v1-v1.19.33.zip` | GOAMD64=v1 | 最高 | 中 | 显式 v1，等价于 compatible 的指令集档 |
| `mihomo-windows-amd64-v2-v1.19.33.zip` | GOAMD64=v2 | 中（需 SSE4.2/POPCNT 等） | 中 | 老 CPU（2010 前/部分 Atom）会 `Illegal Instruction` |
| `mihomo-windows-amd64-v3-v1.19.33.zip` | GOAMD64=v3 | 低（需 AVX2 等） | 中 | 仅新 CPU；崩溃表现为启动即退出无日志 |
| `mihomo-windows-arm64-v1.19.33.zip` | ARM64 | — | — | 非目标平台，代码按 `ARCH` 分支保留 |

**选择策略（`kernel/fetch_core.rs` 实现顺序）**：

1. 按 `std::env::consts::ARCH` 选架构族：`x86_64` → `amd64`；其它 → 返回 `E_UNSUPPORTED`（不静默失败）。
2. **默认下载 `-compatible`**。理由：mihomo 是 IO 密集型转发进程，v1→v3 的指令集收益在用户可感知层面
   远小于「老 CPU 直接崩」的代价；体积差异可忽略。
3. 校验：先查 release 的 `sha256`（发布页正文/校验文件中给出）。**拿不到校验值就记 `WARN` 并继续**，
   同时在 About 页标注「未校验」，禁止假装校验过；**禁止**在没有任何校验的情况下自动重试到成功。
4. 解压：只取 zip 内的 `mihomo.exe`（或 `mihomo-windows-amd64*.exe`），落到
   `%LOCALAPPDATA%\ClashZhengl\runtime\core\<version>\mihomo.exe`（C.3），落盘后立刻校验文件大小 > 1 MB。
5. 记录 `core_version` 到设置；旧版本目录在启动时按「保留当前 + 上一版」清理。
6. 下载失败映射：`E_NET_TIMEOUT` / `E_NET_DNS` / `E_HTTP_STATUS`；磁盘失败 `E_STORE_IO`。

```powershell
# 人工核对某个 release 的资产名与校验值（不写进产品代码，仅供核对）
$rel = Invoke-RestMethod -Headers @{ 'User-Agent' = 'clash-zhengl' } `
    -Uri "https://api.github.com/repos/MetaCubeX/mihomo/releases/tags/v1.19.33"
$rel.assets | Where-Object name -like 'mihomo-windows-amd64*' |
    Select-Object name, size, browser_download_url
```

---

## A.8 启动后健康检查流程（写进 `kernel/supervisor.rs`）

```text
1. spawn mihomo  -d <runtime dir>  -f <runtime/config.yaml>
2. 轮询 GET /version（每 200ms，最多 40 次 = 8s）
   ├─ 200 且 version 非空 → Running，发 core:state
   ├─ 401 → E_CORE_API_UNAUTH（secret 不一致，通常是重建配置时没同步）
   ├─ 连接被拒 → 继续等（内核还在绑定端口）
   └─ 子进程已退出 或 超时 → 读内核 stderr 尾部 200 行 → E_CORE_START_FAILED
3. 额外核对 GET /configs 的 mixed-port 是否等于期望值；不等 → 视为端口冲突，重启并重新分配
4. 记录 core ready 日志锚点（供 10 章启动耗时测量）：
   INFO [kernel] ready pid=1234 api=127.0.0.1:49152 mixed=10801 elapsed_ms=612
```

---

## A.9 常见坑（按踩坑频率排序）

| # | 坑 | 现象 | 正确做法 |
| --- | --- | --- | --- |
| 1 | **unix socket / named pipe 通道不校验 secret** | 用 `external-controller-unix` 或 Windows named pipe（`\\.\pipe\...`）方式接入时，鉴权中间件与 TCP 通道行为不一致，请求可能不带 `Authorization` 也通过 | **绝不依赖该通道作为安全边界**；本项目只用 TCP 回环 + Bearer。若必须用管道，仍要带 secret，并在测试中显式验证「不带 secret 必须 401」 |
| 2 | **SAFE_PATHS 内置名受保护** | 对 `DIRECT`/`REJECT`/`REJECT-DROP`/`PASS`/`COMPATIBLE`/`GLOBAL` 及其小写形式做 `PUT`/`DELETE` 返回 `400` | 切换前过滤：目标名命中安全名白名单（不区分大小写）→ 跳过并记 Warn；`rank.rs` 里 `exclude_keywords` 之外再加一层 |
| 3 | **节点名需 URL 编码** | 中文/空格/`&`/`#`/`/` 名称直接拼 URL → 404 或被截断 | 一律 `percent-encode`；Rust 用 `reqwest::Url` + `.path_segments_mut()`，不要字符串拼 |
| 4 | **`external-ui` 与 CORS** | 配置里留了 `external-ui` 会额外起静态文件服务；用浏览器跨域调 API 还会受 CORS 预检影响 | 生产配置**不写** `external-ui`；桌面端用 Rust 直连，不经浏览器，不依赖 CORS |
| 5 | **端口冲突** | `external-controller` 或 `mixed-port` 被占用 → 内核起不来（日志 `bind: Only one usage of each socket address`）或起来了但 `mixed-port` 被内核改成别的值 | 分配前用 `TcpListener::bind` 探测；失败重试 3 次；全失败返回 `E_CORE_PORT_EXHAUSTED`。启动后**必须**用 `GET /configs` 回读核对 |
| 6 | **回环请求被系统代理劫持** | 用户开了系统代理后本地 API 全部超时 | `reqwest` 客户端 `.no_proxy()` |
| 7 | **`/configs` PUT 与 PATCH 混淆** | PATCH 改 `mixed-port` 看似成功，重启后回退 | 端口/密钥类字段只走「重建配置 + 重启」 |
| 8 | **`secret` 里有需要转义的字符** | 拼 log/命令行时串位 | secret 只用 `[0-9a-f]{32}`，不用 base64 |
| 9 | **进程没杀干净** | 托盘退出后 `mihomo.exe` 残留，下次启动端口冲突 | Windows Job Object（`KILL_ON_JOB_CLOSE`）绑定子进程；退出时先 SIGKILL 等价 `taskkill /T /F` |
| 10 | **`log-level` 枚举拼错** | 写 `warn` 内核启动直接报配置错误 | 只接受 `silent/error/warning/info/debug` |

---

## A.10 内核错误响应形态（用于映射到 C.8）

| HTTP | body 示例 | 含义 | 我们的映射 |
| --- | --- | --- | --- |
| `400` | `{"message":"Invalid config"}` | 配置非法 | `E_CORE_START_FAILED` |
| `401` | `{"message":"Unauthorized"}` | secret 不对 | `E_CORE_API_UNAUTH` |
| `404` | `{"message":"Resource not found"}` | 节点/组不存在 | 不计错误码，记 Warn，跳过 |
| `504` | `{"message":"Timeout"}` | 延迟探测超时 | 不计错误码，`ProbeResult.error = "timeout"` |
| 连接失败 | — | 内核未监听/已退出 | `E_CORE_NOT_FOUND` / `E_CORE_START_FAILED` |

**统一处理约定**：
1. 所有内核调用返回 `Result<T, AppError>`，`AppError.detail` 只放 `status` 与**截断到 200 字符**的 body；
2. body 里若出现 `secret`、`Bearer `、节点密码字段，脱敏后再入日志（C.9）；
3. 任何 5xx 都不重试超过 2 次，且退避 200ms/800ms，避免测速阶段放大风暴。

---

## A.11 CLI 参数速查（`kernel/supervisor.rs` 只允许这些）

| 参数 | 说明 | 我们是否使用 |
| --- | --- | --- |
| `-d <home>` | 配置/数据目录 | ✅ 必用，指向 `%LOCALAPPDATA%\ClashZhengl\runtime\` |
| `-f <config>` | 指定配置文件 | ✅ 必用，指向 `runtime/config.yaml`（绝对路径） |
| `-t` | 只做配置测试后退出 | ✅ P2 生成配置后先 `-t` 自检，再启动 |
| `-v` | 打印版本 | ✅ 仅诊断导出 |
| `-secret <s>` | 命令行覆盖 secret | ⛔ 不用（会进进程列表，等于泄露） |
| `-ext-ctl <addr>` | 命令行覆盖 external-controller | ⛔ 不用（配置文件里写更可控） |

```powershell
# 用 -t 做配置自检（P2/P7 通用，返回码 0 表示配置可用）
& "$env:LOCALAPPDATA\ClashZhengl\runtime\core\v1.19.33\mihomo.exe" `
    -d "$env:LOCALAPPDATA\ClashZhengl\runtime" `
    -f "$env:LOCALAPPDATA\ClashZhengl\runtime\config.yaml" -t
"exit=$LASTEXITCODE"   # 0 = 配置合法
```

> **`-secret` 为什么禁用**：Windows 上任何本机用户都能用 `Get-CimInstance Win32_Process` 读到命令行，
> 把 secret 放命令行等于把「仅当前用户可用的回环控制权」降级为「本机任意用户可读」。这是 C.12 的硬红线。

---

## A.12 版本钉死与升级策略

| 项 | 约定 |
| --- | --- |
| 目标版本 | `v1.19.33`（`settings.core_version` 默认值） |
| 升级触发 | 只由用户显式点击「检查更新」，不做后台自动升级 |
| 兼容校验 | 启动后比对 `GET /version` 的 `version` 与目录名；不一致记 Warn 并按目录名为准 |
| 回退 | `runtime/core/` 保留上一版本目录；升级完成后 `-t` 自检失败则自动回退 |
| 禁止 | 不在构建期下载内核、不把内核打进 `src-tauri/binaries` 后再声称「无网络构建」（见 C.12 第 5 条） |

---

## 契约对齐记录（v1.3）

| 契约条目 | 本附录落点 | 状态 |
| --- | --- | --- |
| **v1.3 · §C.6.1 内部接口冻结面**（`SelectorApi`/`ApiError`/`GroupInfo`/`SelectableGroup`/`SwitchRuntimeLocks`） | A.2.3 字段映射 + Rust 结构 + 取数策略末尾的冻结面说明 | ✅ 已落地 |
| **v1.3 · `GroupInfo.kind` 取 API 值 `Selector`（非 YAML 的 `select`）** | A.2 字段表 + **新增「kind / type 取值对照」表** + A.2.1 整表重写 + `is_selector()` | ✅ 已落地（这是最易错点） |
| **v1.3 · `SelectableGroup.members`**（原误写为 `candidates`，并去掉不属于契约的 `hidden`） | A.2.3 Rust 结构 | ✅ 已按 C.6.1 定稿更正 |
| v1.2 · `PUT /proxies/{name}` 只用于 Selector 组 | A.2 表格备注 + A.2.1 专节 + `GroupInfo::is_selector()` | ✅ 已落地 |
| v1.2 · `DELETE /proxies/{name}` 禁用理由 | A.2.2「为什么禁用」表 + 替代方案 | ✅ 已落地 |
| v1.1 · `SwitchRecord.to_delay: Option<u32>` | A.4.1 延迟语义（`delay==0` 视为失败，与 `None` 区分） | ✅ 兼容 |
| v1.1 · `runtime/download/` 续传区 | A.7 解压落点与 C.3 一致 | ✅ 兼容 |
| v1.1 · `RuntimeState.sysproxy` | 不在本附录范围（08 章 / 12 章覆盖） | — |

**未决项**：无。原「`SelectorApi` 零定义」问题已由 C.6.1 解决；本附录的字段定义与 C.6.1 完全一致。
