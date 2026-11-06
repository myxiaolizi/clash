# 附录 C · 工程约定与共享契约（全局唯一权威）

> **用途**：本文件是整套提示词的「宪法」。任何章节、任何 AI 会话的输出与本文冲突时，以本文为准。
> **维护**：只有用户（或 Lead）可以修改本文。编码 Agent 若发现需要变更契约，必须在答复末尾以
> `CONTRACT-CHANGE-REQUEST:` 开头提出理由与建议，**不得擅自改动本文，也不得在代码里悄悄偏离**。
>
> **变更日志**
>
> | 版本 | 日期 | 变更 |
> | --- | --- | --- |
> | v1.0 | 2026-11-06 | 初版 |
> | v1.1 | 2026-11-06 | 裁决 10 条契约缺口：`RuntimeState.sysproxy`；新增 `TrafficSummary`；新增 `core_versions_list` / `switch_history` 命令；新增 `sysproxy:state` / `core:install-progress` 事件；`AppSettings.compliance_accepted_at`（schema_version → 2）；C.3 补 `runtime/sysproxy.journal.json` 与 `runtime/download/`；C.11 明确 12 MB 口径；C.6 `core_install` 语义细化 |
> | v1.2 | 2026-11-06 | 裁决 3 条：`switch_apply_best` 返回 `Option<{primary, all}>`（`primary = all[0]`；无生效组返回 `None`，不存在 `primary: null`）；`SwitchRecord.to_delay` 改 `Option<u32>`；新增 `SwitchPolicy.manual_override_min` 与手动覆盖语义；新增「scope 组必须为 select 类型」约束 |
> | v1.3 | 2026-11-06 | 新增 §C.6.1 内部接口冻结面（`SelectorApi`/`ApiError`/`GroupInfo`/`SelectableGroup`/`SwitchRuntimeLocks`），明确它们**不是 IPC 命令**；`switch_apply_best` 契约式表述定稿；补 `kind` 取值陷阱（API `Selector` ≠ YAML `select`） |
> | v1.4 | 2026-11-06 | 补充 `SelectorApi` 的 **dyn 兼容形式**（boxed-future，`ApiFuture<'a,T>`，不用 `async-trait`）；附录 A 修正 `find-process-mode: strict`（`off` 会让 PROCESS 规则永久失效）、`ipv6: true`、`dns.enable: false`，并新增 A.6.4「禁止生成自动策略组」 |

---

## C.1 产品身份（Identity）

| 项目 | 取值 |
| --- | --- |
| 产品名（小写，用于命令行/包名） | `clash zhengl` |
| 显示名（窗口/托盘/安装包） | `Clash Zhengl` |
| 可执行文件 | `clash-zhengl.exe` |
| Windows 应用标识（Tauri identifier） | `com.clashzhengl.app` |
| 安装包名 | `ClashZhengl_<version>_x64-setup.exe` |
| 版本方案 | SemVer `MAJOR.MINOR.PATCH`，首版 `0.1.0` |
| 目标平台 | Windows 10 1809+ / Windows 11，x64 |
| 语言 | UI 全中文，i18n 键值化（`zh-CN` 默认，预留 `en-US`） |

> ARM64 不是目标平台，但代码中不得硬编码 x64 假设（内核下载逻辑按 `std::env::consts::ARCH` 选择资产）。

---

## C.2 仓库目录结构（Rust + Tauri 2 方案，方案 B 见 02 章节）

```
clash-zhengl/
├─ src-tauri/
│  ├─ Cargo.toml
│  ├─ tauri.conf.json
│  ├─ capabilities/default.json
│  ├─ icons/
│  └─ src/
│     ├─ main.rs                # 入口：单实例、日志、panic hook、tray 初始化
│     ├─ app.rs                 # AppState 组装、生命周期（ready/exit）、优雅退出
│     ├─ commands/              # 唯一的 IPC 入口层（薄，只做校验与转发）
│     │  ├─ mod.rs
│     │  ├─ subscription.rs     # subscription_* 命令
│     │  ├─ profile.rs          # profile_* / 配置预览
│     │  ├─ core.rs             # core_* / 系统代理
│     │  ├─ speedtest.rs        # speedtest_* / switch_*
│     │  ├─ settings.rs         # settings_* / 诊断导出
│     │  └─ dto.rs              # 前端 DTO（camelCase serde rename_all）
│     ├─ domain/                # 纯数据模型与错误，无 IO
│     │  ├─ mod.rs
│     │  ├─ model.rs            # 见 C.5
│     │  ├─ errors.rs           # 见 C.8
│     │  └─ policy.rs           # 切换策略纯函数（可单测，不碰网络）
│     ├─ store/                 # 持久化
│     │  ├─ mod.rs              # Store trait（便于测试注入）
│     │  ├─ json_store.rs       # 原子写 JSON
│     │  ├─ crypto.rs           # Windows DPAPI 加解密
│     │  └─ migrate.rs          # schema_version 迁移
│     ├─ subscription/          # 订阅域
│     │  ├─ mod.rs
│     │  ├─ fetch.rs            # HTTP 拉取（UA/超时/代理/重试）
│     │  ├─ parse.rs            # mihomo config 解析、节点/分组统计、合法性校验
│     │  ├─ userinfo.rs         # subscription-userinfo 头解析 + 网页兜底
│     │  └─ manager.rs          # 多订阅 CRUD、去重、合并成 runtime config
│     ├─ scheduler/             # 定时器
│     │  ├─ mod.rs
│     │  ├─ clock.rs            # 可注入时钟（测试用）
│     │  ├─ due.rs              # 纯函数：给定状态+now → 到期任务集合
│     │  ├─ backoff.rs          # 指数退避 + 抖动
│     │  └─ runner.rs           # 单一 tick 线程，串行调度
│     ├─ speedtest/
│     │  ├─ mod.rs
│     │  ├─ engine.rs           # 并发测速引擎（受信号量约束）
│     │  ├─ probe.rs            # 单节点探测（mihomo /proxies/{key}/delay）
│     │  ├─ rank.rs             # 纯函数：结果 → 候选排序（含排除规则）
│     │  └─ selector.rs         # 应用切换（PUT /proxies/{group}）+ 防抖
│     ├─ kernel/                # mihomo 内核
│     │  ├─ mod.rs
│     │  ├─ supervisor.rs       # 子进程生命周期、健康检查、崩溃重启
│     │  ├─ api.rs              # external-controller REST 客户端
│     │  ├─ ports.rs            # 端口探测与分配
│     │  └─ fetch_core.rs       # 内核下载/校验/解压/版本管理
│     ├─ system/                # 与 OS 交互
│     │  ├─ mod.rs
│     │  ├─ paths.rs            # 应用目录、日志目录、临时目录
│     │  ├─ sysproxy.rs         # WinINET 系统代理开关 + 刷新
│     │  ├─ autostart.rs        # 开机自启（注册表 Run 或计划任务）
│     │  ├─ notify.rs           # 系统通知（失败降级为托盘气泡）
│     │  ├─ tray.rs             # 托盘菜单构建与事件
│     │  └─ single_instance.rs  # 命名互斥 + 唤起已有窗口
│     └─ util/
│        ├─ log.rs              # tracing 初始化、脱敏、滚动
│        ├─ time.rs
│        └─ net.rs              # 代理客户端构造、UA 常量
├─ src/                         # 前端（TypeScript，无需重型框架）
│  ├─ main.ts
│  ├─ ipc/client.ts             # invoke 封装 + 错误归一
│  ├─ ipc/events.ts             # listen 事件订阅
│  ├─ ui/…                      # 订阅列表 / 流量卡 / 节点测速 / 设置 / 日志
│  └─ i18n/zh-CN.json
├─ scripts/
│  ├─ fetch-mihomo.ps1          # 下载内核（支持 --proxy）
│  ├─ dev.ps1 / build.ps1
│  └─ perf-probe.ps1            # 采集内存/CPU/启动耗时
├─ docs/
└─ tests/                       # 集成测试 + Mock 订阅服务器
```

> **分层铁律**：`commands` 只做参数校验与 DTO 转换；业务在 `subscription/speedtest/kernel/scheduler`；
> IO 与纯逻辑分离，纯函数（`domain/policy.rs`、`scheduler/due.rs`、`speedtest/rank.rs`）必须有单测。

---

## C.3 运行时路径与文件格式

根目录（Roaming，随用户漫游）：`%APPDATA%\ClashZhengl\`
缓存/大文件（Local，不漫游）：`%LOCALAPPDATA%\ClashZhengl\`

| 文件 | 位置 | 说明 |
| --- | --- | --- |
| `settings.json` | Roaming | `AppSettings`，含加密字段；原子写 |
| `profiles/index.json` | Roaming | `Vec<Subscription>`（URL 为密文 + 展示用 hint） |
| `profiles/<uid>.yaml` | Roaming | 订阅原始配置（HTTP 响应体原样保存） |
| `profiles/<uid>.meta.json` | Roaming | 该订阅最近一次 UserInfo + ETag/Last-Modified/Content-Length |
| `runtime/config.yaml` | Local | 合并后交给内核的配置（每次启动/更新后重建） |
| `runtime/secret` | Local | 随机 32 hex，`external-controller` 的 secret；ACL 仅当前用户 |
| `runtime/core/<version>/` | Local | 内核可执行文件（按版本分子目录，切换版本靠目录名 + settings 指针） |
| `runtime/download/` | Local | 内核下载临时区（`.part` 与 `.part.json` 续传元数据），完成后清理 |
| `runtime/sysproxy.journal.json` | Local | 系统代理「开启前快照 + 正常退出标记」，用于崩溃后恢复守卫（见 08 章） |
| `runtime/switch-history.json` | Local | `Vec<SwitchRecord>`，仅保留最近 200 条，滚动丢弃 |
| `logs/app-YYYYMMDD.log` | Local | 滚动日志，单文件 2 MB × 保留 3 天 |
| `logs/core-YYYYMMDD.log` | Local | 内核 stdout/stderr |
| `diagnostics/` | Local | 一键诊断导出 zip（脱敏后） |

约束：
- **所有 JSON 写入必须原子**（写 `*.tmp` → `fsync` → `rename`），崩溃不得留下半截文件。
- 读取损坏时：重命名为 `*.corrupt-<ts>`，回退默认值并在 UI 提示，**不得崩溃**。
- 订阅 URL、内核 secret 落盘前用 **Windows DPAPI**（`CryptProtectData`，`CRYPTPROTECT_UI_FORBIDDEN`）加密；
  密钥绑定当前用户。内存中仅在需要发起请求的瞬间解密。

---

## C.4 命名规范

| 层 | 规则 | 示例 |
| --- | --- | --- |
| Rust 模块/文件 | `snake_case` | `fetch_core.rs` |
| Rust 类型/枚举 | `PascalCase` | `Subscription`、`ProxyKind` |
| Rust 函数/变量 | `snake_case` | `parse_userinfo_header` |
| TS 变量/函数 | `camelCase` | `refreshSubscription` |
| IPC 命令 | `<domain>_<action>`，全小写蛇形 | `subscription_list`、`speedtest_start` |
| 前端事件名 | `<domain>:<event>` | `speedtest:progress` |
| localStorage / DOM id | `kebab-case` | `traffic-card` |
| 文档文件 | `NN-kebab-case-title.md` | `06-scheduler-and-timers.md` |

IPC 命令接收的参数与返回体统一 `camelCase`（Rust 侧 `#[serde(rename_all = "camelCase")]`）。

---

## C.5 数据模型（唯一权威定义）

> Rust 为真源，TS 类型由 `src/ipc/types.ts` 手写保持一致；字段新增必须给默认值并升 `schema_version`。

```rust
// ---------- 设置 ----------
pub struct AppSettings {
    pub schema_version: u32,          // 当前 2（v1.1：新增 compliance_accepted_at）
    pub locale: String,               // "zh-CN"
    pub theme: Theme,                 // System | Light | Dark
    pub auto_launch: bool,            // 开机自启，默认 false
    pub silent_start: bool,           // 自启时只进托盘，默认 true
    pub start_core_on_launch: bool,   // 默认 true
    pub compliance_accepted_at: Option<i64>, // 首次合规提示的接受时间；None = 未接受，必须再弹
    pub proxy_mode: ProxyMode,        // SystemProxy | Tun | Manual
    pub api_port: u16,                // 0 = 自动分配（推荐）；显式设置时冲突需报 E_CORE_PORT_EXHAUSTED
    pub bind_address: String,         // 固定 "127.0.0.1"
    pub core_version: Option<String>, // 期望内核版本；None = 已安装的最新
    pub core_download_proxy: Option<String>, // 如 "http://127.0.0.1:1886"，仅用于下载内核
    pub sub_default_interval_min: u32,// 新订阅默认周期，默认 360
    pub sub_fetch_concurrency: u8,    // 默认 2，上限 4
    pub sub_fetch_timeout_ms: u32,    // 默认 15_000
    pub sub_fetch_proxy: Option<String>, // 拉订阅时使用的 HTTP 代理，默认 None
    pub sub_user_agent: String,       // 默认 "clash.meta/1.19.0"（部分机场按 UA 下发不同配置）
    pub speedtest_enabled: bool,      // 默认 true
    pub speedtest_interval_min: u32,  // 默认 30，下限 5
    pub speedtest_probe_url: String,  // 默认 "http://www.gstatic.com/generate_204"
    pub speedtest_timeout_ms: u32,    // 默认 3000，范围 1000..10000
    pub speedtest_concurrency: u8,    // 默认 8，上限 16
    pub switch_policy: SwitchPolicy,
    pub notify_on_switch: bool,       // 默认 false（避免噪音）
    pub notify_on_expire: bool,       // 默认 true
    pub traffic_warn_percent: u8,     // 默认 80，0 = 关闭
    pub log_level: LogLevel,          // Error | Warn | Info | Debug
    pub close_to_tray: bool,          // 默认 true
}

pub enum Theme { System, Light, Dark }
pub enum ProxyMode { SystemProxy, Tun, Manual }
pub enum LogLevel { Error, Warn, Info, Debug }

// ---------- 切换策略 ----------
pub struct SwitchPolicy {
    pub enabled: bool,               // 默认 true
    pub scope: SwitchScope,          // Group(name) 推荐 | Global
    pub tolerance_ms: u32,           // 默认 30：新节点须比当前快 ≥30ms 才切换（防抖）
    pub min_dwell_sec: u32,          // 默认 300：切换后最短驻留时间
    pub failure_threshold: u8,       // 默认 2：连续 N 次探测失败才降级
    pub manual_override_min: u32,    // 默认 60：用户手动选节点后，该组暂停自动切换的分钟数（0 = 不暂停）
    pub exclude_keywords: Vec<String>, // 默认 ["到期","过期","剩余","流量","官网","套餐","重置","客服","域名"]
    pub prefer_region_priority: Vec<String>, // 可选：["香港","HK","日本","JP","新加坡","SG"]，为空则不参与
}
pub enum SwitchScope { Global, Group(String) }

// ---------- 订阅 ----------
pub struct Subscription {
    pub uid: String,                 // 8 位 base32，生成后不可变
    pub name: String,
    pub url_cipher: String,          // DPAPI 密文（base64）
    pub url_hint: String,            // 脱敏展示：保留 scheme+host+path，token 段替换为 ******
    pub enabled: bool,
    pub update_interval_min: u32,    // 每订阅独立，默认取 settings.sub_default_interval_min
    pub last_update_at: Option<i64>, // Unix 秒
    pub last_update_status: UpdateStatus,
    pub last_error: Option<String>,  // 结构化错误码 + 可读信息
    pub mgmt: Option<UserInfo>,
    pub node_count: u32,
    pub group_names: Vec<String>,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

pub enum UpdateStatus { Never, Ok, Failed }

pub struct UserInfo {
    pub upload: Option<i64>,         // 字节
    pub download: Option<i64>,       // 字节
    pub total: Option<i64>,          // 字节；0 表示不限量
    pub expire_at: Option<i64>,      // Unix 秒；None 表示无限期
    pub web_page_url: Option<String>,
    pub fetched_at: i64,
    pub source: UserInfoSource,      // Header | WebPage | Manual | Unknown
    pub raw: Option<String>,         // 原始 subscription-userinfo 字符串，便于排错
}

pub enum UserInfoSource { Header, WebPage, Manual, Unknown }

// ---------- 测速 ----------
pub struct SpeedtestRun {
    pub id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub probe_url: String,
    pub timeout_ms: u32,
    pub total: u32,
    pub finished: u32,
    pub results: Vec<ProbeResult>,
    pub best: Option<String>,        // node_key
    pub trigger: RunTrigger,         // Timer | Manual | AfterUpdate
}

pub enum RunTrigger { Timer, Manual, AfterUpdate }

pub struct ProbeResult {
    pub node_key: String,            // 形如 "HongKong-01" 或 "sub-a/HongKong-01"（冲突时加前缀）
    pub group: String,               // 所在策略组
    pub kind: ProxyKind,             // Ss | Vmess | Trojan | Hysteria2 | Tuic | Wireguard | Other
    pub delay_ms: Option<u32>,
    pub error: Option<String>,       // 失败原因（超时/拒绝/HTTP 5xx）
    pub tested_at: i64,
}

pub enum ProxyKind { Ss, Vmess, Vless, Trojan, Hysteria2, Tuic, Wireguard, Http, Socks5, Other }

// ---------- 运行态与事件 ----------
pub struct RuntimeState {
    pub core: CoreState,             // Stopped | Starting | Running | Failed(reason)
    pub api_endpoint: Option<String>,
    pub active_profile_uid: Option<String>,
    pub selected: Vec<SelectedNode>, // 每个 selector 组当前选中 + 实测延迟
    pub traffic: TrafficCounter,     // 进程级上下行累计 + 速率
    pub sysproxy: SysProxyState,     // 系统代理当前状态（UI 顶栏/托盘开关的初始化依据）
    pub started_at: i64,
}

pub enum CoreState { Stopped, Starting, Running, Stopping, Failed(String) }

/// 系统代理状态。`enabled` 是「我们写入的」还是「用户原本的」必须区分，
/// 否则崩溃恢复会误关用户自己配的代理。
pub struct SysProxyState {
    pub enabled: bool,               // 注册表当前 ProxyEnable 的实际值
    pub server: Option<String>,      // 当前 ProxyServer 值（如 "127.0.0.1:7897"）
    pub managed_by_us: bool,         // true = 该值由本应用写入且尚未恢复
    pub port: Option<u16>,           // managed_by_us 时，指向的本地混合端口
}

pub struct SelectedNode { pub group: String, pub node: String, pub delay_ms: Option<u32> }

pub struct TrafficCounter { pub up_bytes: u64, pub down_bytes: u64, pub up_rate: u64, pub down_rate: u64, pub at: i64 }

/// 流量页数据源（`traffic_summary` 的返回体）。
/// 聚合规则：`total`/`expire_at` 取「最保守值」——total 取各订阅之和（含 0 视为不限量则忽略该订阅），
/// expire_at 取最早的非 None；`upload`/`download` 为各订阅之和（缺失项按 0 参与求和，但 `has_missing` 置 true）。
pub struct TrafficSummary {
    pub items: Vec<SubscriptionTraffic>, // 每个启用订阅一条，按 sort_order
    pub aggregate: AggregateTraffic,     // 全部启用订阅的聚合
    pub at: i64,
}

pub struct SubscriptionTraffic {
    pub uid: String,
    pub name: String,
    pub upload: Option<i64>,
    pub download: Option<i64>,
    pub total: Option<i64>,          // 0 = 不限量
    pub expire_at: Option<i64>,      // None = 无限期
    pub used: Option<i64>,           // 派生态：upload+download，任一缺失即 None
    pub used_percent: Option<f32>,   // 派生态：total > 0 时才有值
    pub days_left: Option<i64>,      // 派生态：expire_at - now，可为负
    pub source: UserInfoSource,
    pub stale: bool,                 // 距 fetched_at 超过 2 个更新周期 → UI 显示「数据可能过期」
}

pub struct AggregateTraffic {
    pub upload: i64,
    pub download: i64,
    pub total: Option<i64>,          // 全部 total 均为 0/缺失 → None（不限量）
    pub expire_at: Option<i64>,      // 最早到期
    pub used: i64,
    pub used_percent: Option<f32>,
    pub subscription_count: u32,
    pub has_missing: bool,           // 有订阅缺少 userinfo，聚合数字仅为下限
}


pub struct SwitchRecord {
    pub at: i64,
    pub group: String,
    pub from_node: Option<String>,
    pub to_node: String,
    pub from_delay: Option<u32>,
    pub to_delay: Option<u32>,       // Manual 切换时可能无新鲜测速数据 → None（不得用 0 冒充）
    pub reason: SwitchReason,        // Best | CurrentFailed | Manual | GroupMissing
}
pub enum SwitchReason { Best, CurrentFailed, Manual, GroupMissing }
```

**手动覆盖（manual override）语义**

`switch_apply_node` 成功后，该组进入「手动覆盖」状态，`manual_override_min` 分钟内自动切换**跳过该组**：

- 覆盖状态是**内存态**，不落盘；应用重启后清空（重启后用户预期恢复自动策略）。
- 结构：`SwitchRuntimeLocks { group: String, locked_at: i64 }`，存放于 `speedtest/selector.rs` 的
  运行态，不进 `RuntimeState`（避免每 2s 的 `traffic:tick` 负载里夹带无关字段）。
- 到期判定：`now - locked_at >= manual_override_min * 60` → 解除，并在下一次测速时恢复参与排序。
- `manual_override_min = 0` 表示完全不做手动覆盖（用户选了就立刻可被自动切换覆盖）。
- 用户再次手动选择同一组 → 以最后一次时间续期。

**合并配置的组类型约束（对 04 章的要求）**

`switch_policy.scope` 指向的策略组**必须**生成为 `type: select`。
禁止同时生成同名 `type: url-test` / `fallback` 组 —— 否则内核自带健康检查会与本应用的
定时测速争夺"当前节点"，表现为节点来回跳。自动切换由本应用**显式** `PUT /proxies/{group}` 完成。

**派生字段规则（前端只展示，不重算）**
- 已用 = `upload + download`（任一缺失则显示 `—`）
- 剩余 = `total - 已用`（`total == 0` 或缺失则显示「不限量」/`—`）
- 使用率 = `已用 / total`，`total == 0` 时不计算
- 剩余天数 = `expire_at - now`，负数表示已过期；`expire_at == None` 显示「无限期」
- 单位：字节内部一律 `i64/u64`，展示用 `B/KB/MB/GB/TB`，1024 进制，保留 2 位小数

---

## C.6 IPC 命令表（Tauri `invoke`）

| 命令 | 入参 | 返回 | 说明 |
| --- | --- | --- | --- |
| `app_bootstrap` | — | `{settings, subscriptions, runtime}` | 首屏一次性拉取，禁多次往返 |
| `settings_get` / `settings_save` | `/ AppSettings` | `AppSettings` | 保存后按需 hot-apply（见 08 章节） |
| `subscription_list` | — | `Vec<Subscription>` | |
| `subscription_add` | `{name, url, intervalMin?}` | `Subscription` | 立即拉取一次，失败也登记但标记 Failed |
| `subscription_update_url` | `{uid, url?}` | `Subscription` | url 为空只改元信息 |
| `subscription_rename` | `{uid, name}` | `Subscription` | |
| `subscription_set_enabled` | `{uid, enabled}` | `Subscription` | 停用则不参与合并与测速 |
| `subscription_set_interval` | `{uid, intervalMin}` | `Subscription` | 下限 15 分钟 |
| `subscription_remove` | `{uid}` | `()` | 删除该 uid 的 yaml/meta/index 记录 |
| `subscription_refresh` | `{uid}` | `Subscription` | 强制刷新，忽略周期 |
| `subscription_refresh_all` | — | `Vec<Subscription>` | 串行限流 |
| `subscription_reveal_url` | `{uid}` | `{url}` | 仅用户点击「显示」时调用，不落日志 |
| `traffic_summary` | — | `TrafficSummary` | 多订阅聚合 + 单订阅明细 |
| `speedtest_start` | `{trigger?}` | `{runId}` | 已在跑则返回当前 runId（幂等） |
| `speedtest_status` | `{runId}` | `SpeedtestRun` | 前端 1s 轮询或走事件 |
| `speedtest_cancel` | `{runId}` | `()` | 取消未开始的探测 |
| `switch_apply_best` | `{runId}` | `Some({primary, all}) \| None` | 应用到 scope 内的 selector 组。**返回 `None`（而非空 primary）表示本轮没有任何组被切换**；有生效时 `primary = all[0]` = **首个实际生效的组**（不得指向被抑制/未生效的组），`all` 按实际生效顺序排列（`scope=Group` 时长度 1）。`primary` 与 `all[0]` 恒等，**不存在 `primary: null` 的形态** |
| `switch_apply_node` | `{group, node}` | `SwitchRecord` | 手动指定节点；成功后该组进入手动覆盖期（见 C.5 `manual_override_min`） |
| `core_status` | — | `RuntimeState` | |
| `core_start` / `core_stop` / `core_restart` | — | `RuntimeState` | 幂等 |
| `core_install` | `{version?}` | `{version}` | 下载并安装内核；version 为空=最新稳定版。须发 `core:install-progress` |
| `core_versions_list` | — | `{installed: Vec<String>, current: Option<String>, available: Vec<String>}` | 设置页版本下拉；`available` 来自 GitHub Release（需网络，失败返回空数组 + 不报错） |
| `sysproxy_set` | `{enabled}` | `SysProxyState` | 系统代理开关；返回实际生效状态（可能被策略/权限拒绝） |
| `profile_preview` | `{uid}` | `{yaml}` | 只读预览原始配置（脱敏节点凭据） |
| `switch_history` | `{limit?}` | `Vec<SwitchRecord>` | 最近切换记录，按时间倒序，limit 默认 50、上限 200 |
| `logs_tail` | `{lines}` | `{text}` | 默认 200 行，上限 2000 |
| `diagnostics_export` | — | `{path}` | 生成脱敏 zip 并返回路径 |

> **不做 `ui_visibility_set`**：窗口可见性由 Rust 侧监听 Tauri 窗口事件（`WindowEvent::Focused` /
> `Resized` / 最小化）自行判断，不新增 IPC。前端不得为降采样再发心跳。

### C.6.1 内部接口（不是 IPC，但属于契约冻结面）

以下不是前端可调用的命令，但**跨章节共享**，签名必须在各章一致（明细表见 07 章 §7.16）：

| 接口 | 定义位置 | 说明 |
| --- | --- | --- |
| `trait SelectorApi` | `kernel/api.rs`（08 章实现，07 章消费） | `proxies_selectable` / `proxy_group` / `set_group_selected`；**不是 IPC 命令，不得为它新增 invoke** |
| `enum ApiError` | `kernel/api.rs` | `Unauthorized` / `NotFound(String)` / `Timeout` / `Status(u16, String)` / `Transport(String)`；→ `AppError` 映射见 08 章 |
| `struct GroupInfo` | `kernel/api.rs` | `{name, kind, now: Option<String>, all: Vec<String>, test_url: Option<String>, hidden: bool}` |
| `struct SelectableGroup` | `kernel/api.rs` | `{name, members: Vec<String>, now: Option<String>}`（**字段名是 `members`，不是 `candidates`**；不含 `hidden`，隐藏组在 `proxies_selectable` 里就被过滤掉） |
| `struct SwitchRuntimeLocks` | `speedtest/selector.rs` | `{group, locked_at}`，纯内存态，**不落盘、不进 `RuntimeState`** |

> 这五个类型/接口一旦被 07 §7.16 冻结表收录，任何章节改动都必须同步该表并升冻结版本号。

**`SelectorApi` 的 dyn 兼容形式（必须照此实现，否则 07 章假实现无法注入）**

Rust 1.85 下 `async fn in trait` **默认不是 dyn 兼容**的，而 07 章需要用 `Arc<dyn SelectorApi>` 注入
`FakeApi`。因此本 trait 一律使用 **boxed-future 等价形式**，不写裸 `async fn`：

```rust
use std::future::Future;
use std::pin::Pin;

pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

pub trait SelectorApi: Send + Sync {
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>>;
    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>;
    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()>;
}
```

- 实现侧写 `fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>> { Box::pin(async move { … }) }`；
- 调用侧写法与 `async fn` 完全一致（`.await` 即可），无额外成本（一次装箱，每请求一次，可忽略）；
- **不用 `async-trait` 宏**：它等价于 boxed future 但多一层宏依赖，且本项目拒绝为它扩大依赖白名单。

#### ⚠️ `kind` 的取值陷阱（写错会导致自动切换完全失效）

`GroupInfo.kind` / `SelectableGroup` 的筛选依据是**内核 REST API 返回的 `type` 原值**，
与订阅 YAML 里写的 `type:` **大小写不同**，两者不可混用：

| 来源 | 取值 | 用途 |
| --- | --- | --- |
| 订阅 YAML（机场下发的原始配置） | `select` / `url-test` / `fallback` / `load-balance`（全小写、连字符） | 我们用 `serde_yaml` 读写配置时判断 |
| 内核 API `/proxies`、`/group` 的 `type` 字段 | `Selector` / `URLTest` / `Fallback` / `LoadBalance`（大驼峰、无连字符） | `SelectorApi` 返回值里一律用它 |

**规则**：
- `GroupInfo.kind` / `SelectableGroup` 筛选**只接受 API 值** `"Selector"`；
- 比较必须**大小写敏感且精确匹配**，禁止 `contains("select")`、`to_lowercase() == "select"` 这类写法
  （`"Selector"` 转小写是 `"selector"`，与 YAML 的 `"select"` 并不相等，这正是最容易写错的地方）；
- 两套取值的映射表见附录 A「kind/type 取值对照表」；
- 单测必须包含一条断言：对 `kind == "Selector"` 返回 true、对 `"select"`（错误写法传入）返回 false，
  以保证将来没人"顺手改回小写"。

## C.7 事件表（后端 → 前端）

| 事件 | 负载 | 触发 |
| --- | --- | --- |
| `core:state` | `CoreState` | 内核状态变化 |
| `core:install-progress` | `{version, phase, received, total, percent}` | 内核下载/校验/解压进度；`phase` ∈ `download\|verify\|extract\|done`；合并到 ≤2 条/秒 |
| `sysproxy:state` | `SysProxyState` | 系统代理状态变化（含被外部程序改动后探测到） |
| `subscription:updated` | `Subscription` | 单个订阅刷新成功/失败 |
| `subscription:progress` | `{uid, phase, percent?}` | 拉取/解析进度（可选） |
| `traffic:tick` | `TrafficCounter` | 运行期每 2s（窗口不可见时降到 10s 或暂停） |
| `speedtest:progress` | `{runId, finished, total, latest: ProbeResult}` | 每完成 1 个探测（批量合并，≤4 条/秒） |
| `speedtest:done` | `SpeedtestRun` | 全量完成 |
| `switch:applied` | `SwitchRecord` | 自动或手动切换成功 |
| `notify` | `{level, title, body}` | 需要浮出的提示 |

事件限流要求：任何事件频率 > 5 Hz 必须在后端合并；前端不得因事件风暴重渲染列表（见 09 章节）。
单条事件负载 > 8 KB 视为设计错误（如 `speedtest:progress` 不得携带全量 results）。

---

## C.8 错误码（`AppError` 的 `code` 字段）

| code | 场景 | 用户可见文案（中文） |
| --- | --- | --- |
| `E_NET_DNS` | 域名解析失败 | 无法解析订阅域名，请检查网络或 DNS |
| `E_NET_TIMEOUT` | 连接/读取超时 | 连接订阅服务器超时 |
| `E_NET_TLS` | 证书/握手失败 | 订阅服务器 TLS 校验失败 |
| `E_NET_PROXY` | 代理不可用 | 拉取代理（如 127.0.0.1:1886）不可用 |
| `E_HTTP_STATUS` | 非 2xx | 订阅服务器返回 HTTP {status} |
| `E_PARSE_YAML` | YAML 非法/缺 `proxies` | 订阅内容不是有效的 Clash 配置 |
| `E_PARSE_EMPTY` | 节点数为 0 | 订阅中没有可用节点，请检查订阅是否为空或已过期 |
| `E_USERINFO_MISSING` | 无 `subscription-userinfo` 且网页兜底失败 | 该订阅未提供流量信息 |
| `E_CORE_NOT_FOUND` | 内核不存在 | 未安装内核，请先下载 |
| `E_CORE_START_FAILED` | 启动失败/端口占用 | 内核启动失败：{detail} |
| `E_CORE_API_UNAUTH` | 401 | 内核 API 鉴权失败（secret 不一致） |
| `E_CORE_PORT_EXHAUSTED` | 端口分配失败 | 无可用本地端口 |
| `E_STORE_IO` | 读写失败 | 配置写入失败，请检查磁盘权限 |
| `E_STORE_DECRYPT` | DPAPI 解密失败（换了用户/机器） | 订阅地址无法解密，请重新填写 |
| `E_PERM_ADMIN` | TUN 等需管理员 | 该功能需要以管理员身份运行 |
| `E_UNSUPPORTED` | 平台不支持 | 当前系统不支持该功能 |
| `E_BUSY` | 同一资源并发操作 | 正在执行中，请稍候 |

错误对象结构统一：`{ code: string, message: string, detail?: string, retryable: boolean }`。
**禁止**把原始 URL、token、secret、节点密码放进 `message/detail/日志`。

---

## C.9 日志规范

单行结构：`<RFC3339 本地时间> <LEVEL> [<module>] <message> key=value ...`

```
2026-11-06T15:54:07.123+08:00 INFO  [scheduler] tick tasks=3 next_in=42s
2026-11-06T15:54:09.001 WARN  [subscription] fetch failed uid=a1b2c3d4 code=E_NET_TIMEOUT retry=1/3
2026-11-06T15:54:12.500 INFO  [speedtest] done total=128 ok=121 best=HongKong-01 delay=42ms
```

- 级别使用：Error 只用于需要用户介入或数据损坏；Warn 用于可恢复失败；Info 用于状态迁移；
  Debug 用于每个网络请求（默认关闭，可在设置中开启）。
- **脱敏器必须在 tracing 层实现**：正则替换 `token|access_token|key|secret|passwd|password|auth`
  的查询参数值；订阅命令行的 `--proxy` 也需脱敏（若含凭据）。
- 内核 stdout 单独文件，且过滤其中的 `secret` 打印。

---

## C.10 时间、单位、精度

| 语义 | 约定 |
| --- | --- |
| 落盘时间 | Unix 秒（`i64`），UTC |
| 展示 | 本地时区 `yyyy-MM-dd HH:mm`（`expire_at` 显示为日期 `yyyy-MM-dd`） |
| 流量 | 字节（`i64/u64`），展示 1024 进制、2 位小数 |
| 速率 | 字节/秒 |
| 延迟 | 毫秒（`u32`），小于 1ms 记 0 |
| 定时器精度 | ±10%，最短周期：订阅 15 分钟、测速 5 分钟 |
| 抖动 | 所有周期性任务启动时加 `0..30s` 随机偏移，避免同刻并发 |

---

## C.11 性能预算（硬指标，验收即量化）

| 指标 | 预算 | 测量方法 |
| --- | --- | --- |
| 应用进程 RSS（窗口隐藏、内核运行中） | ≤ 90 MB | `perf-probe.ps1` 采样 60s 取 P95 |
| 应用进程 RSS（窗口可见） | ≤ 140 MB | 同上 |
| 空闲 CPU（5 分钟平均） | ≤ 1.5%（单核占比） | 同上 |
| 测速期间 CPU | ≤ 25% 单核且持续时间 ≤ 5s | 手动 + 采样 |
| 冷启动到托盘可用 | ≤ 1.2s | `Stopwatch`，取 5 次中位数 |
| 含内核代理可用 | ≤ 3s | 同上 |
| 安装包体积 | ≤ 12 MB（口径：**签名后的 `ClashZhengl_<ver>_x64-setup.exe`**，不含内核与 WebView2） | 构建产物 `Get-Item .Length` |
| 常驻磁盘占用 | ≤ 120 MB（口径：安装目录 + `%LOCALAPPDATA%` 下运行数据，不含 `%APPDATA%` 里的订阅原文） | 目录统计 |
| 后台唤醒 | 空闲时不产生 < 30s 的定时唤醒 | trace 采样 |

不达标时的处置顺序见 10 章节（先降采样频率 → 关内核按需启动 → 换 UI 方案）。

> **口径争议的仲裁**：若实测在预算的 100%–120% 之间，允许保留并记入 `docs/perf-log.md`，
> 但下一阶段必须给出改进项；超过 120% 视为不达标，必须立即处置。

---

## C.12 安全与合规基线

1. **凭据**：订阅 URL 含 token，属高敏数据。落盘 DPAPI 加密；UI 默认打码；日志与诊断包全脱敏。
2. **本地端口**：`external-controller` 必须 `127.0.0.1` + 随机 secret（32 hex，每次启动或每次 config 重建）。
   禁止 `0.0.0.0`、禁止空 secret。
3. **网络**：只访问用户配置的订阅地址、探测 URL、内核下载地址；**不得内置遥测、不得回传任何数据**。
4. **权限**：默认非管理员运行；TUN/系统代理若需提权，必须由用户显式点击并说明原因。
5. **内核**：mihomo 为 **GPL-3.0**。以独立进程（sidecar）调用，不修改其源码，不静态链接。
   分发策略二选一，并在 About 页声明：
   - (a) 安装包**不捆绑**内核，首次启动引导用户确认后从官方 Release 下载（推荐，规避 GPL 传染与体积）；
   - (b) 捆绑内核则必须附带 mihomo 的 LICENSE、版本与源码获取链接，并在安装包内提供其源码可获取声明。
6. **不提供任何节点/订阅**：应用本身不含代理服务，仅管理用户自有的订阅配置，首次启动显示合规提示。
7. **卸载**：卸载时询问是否保留 `%APPDATA%\ClashZhengl`（含加密数据），默认保留。

---

## C.13 代码质量门禁

提交前必须全绿（CI 与本地一致）：

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
pnpm tsc --noEmit      # 若使用前端
pnpm biome check       # 若使用前端
cargo tauri build --no-bundle   # 编译通过
```

- 禁止 `unwrap()/expect()` 进入生产路径（除启动期不可恢复错误，需 `expect` 带明确文案）。
- 网络 IO 必须带超时；所有 `async` 任务必须有取消路径或超时，禁止裸 spawn 无人回收。
- 新增依赖需说明理由与体积影响（见 02 章节依赖白名单）。

---

## C.14 术语表

| 术语 | 含义 |
| --- | --- |
| 订阅 / subscription | 机场提供的带 token 的 URL，返回 Clash/mihomo YAML |
| profile | 订阅下载到本地后的配置文件 |
| 节点 / proxy | YAML `proxies[]` 中的一条代理 |
| 策略组 / proxy-group | `proxy-groups[]`，如 `select`、`url-test`、`fallback` |
| selector | `type: select` 的组，可通过 API `PUT /proxies/{group}` 手动切换 |
| UserInfo | 订阅响应头 `subscription-userinfo` 中的流量与到期信息 |
| external-controller | 内核暴露的 REST API 监听地址（本机回环） |
| TUN | 虚拟网卡全局接管流量，需管理员与 wintun 驱动 |
| 延迟 / delay | 通过内核 `/proxies/{name}/delay` 探测的握手耗时（ms） |
