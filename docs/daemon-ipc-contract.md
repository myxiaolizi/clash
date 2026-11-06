# daemon ↔ WinUI IPC 契约 v1

本文件是 Rust daemon 与 C# WinUI 3 前端之间的**唯一接口约定**，两侧各自实现，不得单方面变更。

## 1. 进程模型

```
ClashZhengl.exe (C#/WinUI 3, GUI)
   │  启动时 spawn 子进程（CREATE_NO_WINDOW）
   ▼
clash-zhengl-daemon.exe (Rust, 无界面)
   ├─ 管理 mihomo.exe 子进程
   ├─ 订阅拉取/解析/配置生成
   ├─ 测速、自动切换、调度器
   └─ 监听 127.0.0.1:<随机端口>
```

- GUI 退出时终止 daemon（并让 daemon 清理 mihomo 与系统代理）。
- GUI 崩溃时 daemon 可继续运行（代理不断），下次 GUI 启动时**复用**已有 daemon。

## 2. 发现 daemon（在 `%LOCALAPPDATA%\ClashZhengl\runtime\`）

| 文件 | 内容 |
|---|---|
| `daemon.port` | 十进制端口号，如 `52341` |
| `daemon.token` | 32 位十六进制随机串，每次 daemon 启动都变 |

启动顺序：
1. 读 `daemon.port` → 若存在，对 `GET /health` 发请求（无需 token）。
2. 健康检查成功 → **复用**该 daemon，跳到第 4 步。
3. 否则 spawn `clash-zhengl-daemon.exe`，轮询 `/health` 最多 15 秒（每 200ms 一次）。
4. 读 `daemon.token`，之后所有 `/rpc` 请求带 `Authorization: Bearer <token>`。

## 3. 请求 / 响应

### `POST /rpc`

请求头：`Authorization: Bearer <token>`、`Content-Type: application/json`

```json
{ "cmd": "subscription_list", "args": {} }
```

成功（HTTP 200）：

```json
{ "ok": true, "data": <任意 JSON> }
```

失败（HTTP 200，`ok=false`）：

```json
{ "ok": false, "error": { "code": "E_NET_OFFLINE", "message": "网络不可用", "detail": null, "retryable": true } }
```

> 业务错误一律 HTTP 200 + `ok=false`，只有鉴权失败返回 401。
> C# 侧应把 `ok=false` 转成异常 `DaemonException(code, message, detail, retryable)`。

### `GET /health`（无鉴权）

```json
{ "ok": true, "data": { "version": "0.1.0" } }
```

### `GET /events`（SSE，需鉴权）

```
data: {"event":"core:state","data":{...}}

```

事件名与 C.6 契约一致。前端按 `event` 字段分发。

## 4. 命令清单

命令名与参数**逐字沿用 C 章契约**（见 `prompts/appendices/C-conventions.md` C.6），
返回体结构见 `src-tauri/src/commands/dto.rs`（serde 默认 camelCase 之外的字段请以该文件 serde 属性为准）。

### app / settings

| cmd | args | 返回 |
|---|---|---|
| `app_info` | `{}` | `AppInfoDto` |
| `app_accept_compliance` | `{}` | `OpResultDto` |
| `app_open_data_dir` | `{}` | `OpResultDto` |
| `app_open_log_dir` | `{}` | `OpResultDto` |
| `settings_get` | `{}` | `SettingsDto` |
| `settings_update` | `SettingsPatch` | `SettingsDto` |

### kernel

| cmd | args | 返回 |
|---|---|---|
| `core_status` | `{}` | `CoreStatusDto` |
| `core_start` | `{}` | `{ok:true}` |
| `core_stop` | `{}` | `{ok:true}` |
| `core_restart` | `{}` | `{ok:true}` |
| `core_list_versions` | `{}` | `InstalledCoreDto[]` |
| `core_install` | `{version?:string}` | `OpResultDto` |

`CoreStatusDto` 在 0.1.1 增加 `installed`(bool)、`installedVersion`。
`version` 在 `settings.core_version` 为空时会回退到已安装的最新版本，
避免「明明装了内核却显示未安装」的误判。

### 系统代理

| cmd | args | 返回 |
|---|---|---|
| `sysproxy_get` | `{}` | `RuntimeDto` |
| `sysproxy_enable` | `{}` | `{ok:true}` |
| `sysproxy_disable` | `{}` | `{ok:true}` |
| `runtime_get` | `{}` | `RuntimeDto` |

### 订阅

| cmd | args | 返回 |
|---|---|---|
| `subscription_list` | `{}` | `SubscriptionDto[]` |
| `subscription_add` | `{name,url}` | `SubscriptionDto` |
| `subscription_add_ex` | `{name,url,username?,password?,homepage?,remark?}` | `SubscriptionDto` |
| `subscription_remove` | `{uid}` | `{ok:true}` |
| `subscription_update` | `{uid,name?,enabled?,intervalMin?}` | `SubscriptionDto` |
| `subscription_update_meta` | `{uid,username?,password?,homepage?,remark?}` | `SubscriptionDto` |
| `subscription_reveal_password` | `{uid}` | `string`（明文，仅在用户点「显示密码」时调用） |
| `subscription_fetch` | `{uid,force?}` | `{uid,nodeCount,...}` |
| `subscription_fetch_all` | `{}` | `OpResultDto` |
| `subscription_traffic` | `{}` | `TrafficSummaryDto` |
| `subscription_apply` | `{}` | `OpResultDto` |
| `subscription_preview_config` | `{}` | `string`（YAML 原文） |

`SubscriptionDto` 在 0.1.1 增加 `username`、`hasPassword`(bool)、`homepage`、`remark`。
**密码明文永不随 DTO 返回**，只暴露 `hasPassword` 布尔值。

### 规则 / 分流模式（0.1.1 新增）

| cmd | args | 返回 |
|---|---|---|
| `rule_list` | `{}` | `RuleDto[]` |
| `rule_add` | `{kind,value,policy,enabled?}` | `RuleDto` |
| `rule_update` | `{id,kind?,value?,policy?,enabled?}` | `RuleDto` |
| `rule_remove` | `{id}` | `OpResultDto` |
| `rule_reorder` | `{ids:string[]}` | `RuleDto[]` |
| `mode_get` | `{}` | `string`（`rule` / `global` / `direct`） |
| `mode_set` | `{mode}` | `OpResultDto`（立刻重载内核；内核未运行时存盘待下次生效，仍返回成功） |

- `RuleDto`：`{id, kind, value, policy, enabled, sortOrder, line}`
  - `kind` = `domain` \| `domain_suffix` \| `domain_keyword` \| `ip` \| `process`
  - `policy` = `proxy` \| `direct` \| `block`
  - `line` 为后端生成的 mihomo 规则行原文，可直接展示
- **规则值校验**：禁止逗号、换行、空白字符（逗号可伪造规则字段，换行可注入任意 YAML 节点）。
  IP 类按 IPv4 ≤32 / IPv6 ≤128 校验前缀长度。

**`proxyMode` 与 `mihomoMode` 是两个正交概念，不要混用：**

| 字段 | 含义 | 取值 |
|---|---|---|
| `proxyMode` | 抓流方式 | `system_proxy` \| `tun` \| `manual` |
| `mihomoMode` | 内核分流模式 | `rule` \| `global` \| `direct` |

切换分流模式请用 `mode_set`，不要复用 `settings_update`。

### 流量统计（0.1.1 新增）

| cmd | args | 返回 |
|---|---|---|
| `traffic_stats` | `{}` | `TrafficStatsDto` |
| `traffic_reset_today` | `{}` | `OpResultDto` |

`TrafficStatsDto`：`{todayUp, todayDown, sessionUp, sessionDown, totalUp, totalDown, hourly:[{hour,up,down}]×24, updatedAt}`

- `session*` 为**内核进程**累计（内核重启归零），来自 mihomo API
- `today*` / `total*` 由 daemon 差分持久化（`%APPDATA%\ClashZhengl\traffic-stats.json`，保留 30 天）
- **首次读数只建基线、不计增量** —— 否则会把内核启动前的历史累计一次性算进「今日」，
  用户会看到凭空多出若干 GB

### 节点 / 测速 / 切换

| cmd | args | 返回 |
|---|---|---|
| `node_groups` | `{}` | `GroupDto[]` |
| `node_select` | `{group,node}` | `OpResultDto` |
| `node_group_raw_kind` | `{group}` | `string` |
| `speedtest_run` | `{}` | `SpeedtestStatusDto` |
| `speedtest_status` | `{}` | `SpeedtestStatusDto` |
| `switch_auto_apply` | `{}` | `SwitchRecordDto[]` |
| `switch_history` | `{limit?}` | `SwitchRecordDto[]` |
| `switch_manual` | `{group,node}` | `SwitchRecordDto` |

### 窗口相关（daemon 返回占位，由 GUI 自己处理）

`runtime_show_window`、`window_minimize`、`window_toggle_maximize`、`window_close`
不发给 daemon。

## 5. daemon 生命周期约定

- GUI 正常退出 → 向 daemon 发 `POST /shutdown`（需 Bearer 鉴权，无 body）。
- daemon 收到终止信号 → 关闭系统代理、停止 mihomo、清空 `daemon.port` 与 `daemon.token`。
- daemon 启动时若发现端口文件指向的旧实例仍可连通，则**自己退出**并打印端口号。
- **启动顺序**：先开 HTTP 监听并写出端口文件，再后台执行 `bootstrap()`（加载内核等）。
  这样即便首次启动需要下载内核，GUI 也能立刻连上并显示下载进度，不会一直转圈。

### 内核进程清理

`supervisor::stop()` 用 `taskkill /PID <pid> /T /F` **连子进程树一起杀**。
只用 `start_kill()` 会留下孤儿 mihomo 继续占端口（0.1.0 曾累积 14 个残留进程）。

daemon 启动时还会按 `runtime\core\<version>\mihomo.exe` **精确路径**清理上次异常退出
（任务管理器强杀、断电、崩溃）遗留的内核进程。只清理自己目录下的，
不会碰用户从别处安装的 mihomo。

## 6. 错误码

全部 `E_*` 错误码见 `src-tauri/src/domain/errors.rs` 的 `ErrorCode` 枚举。
GUI 侧只需把 `code` 原样透传给 UI 层做文案映射，不得自行改写。
