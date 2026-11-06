# 13 · 迭代任务卡（P0–P8，可直接复制执行）

> **这是整套提示词的执行入口**。用法：
> 1. 新会话先贴 [01-master-prompt.md](01-master-prompt.md) 全文 + [appendices/C-conventions.md](appendices/C-conventions.md) 全文；
> 2. 再贴**当前阶段**那张卡的「执行指令块」（```text 代码块内全部内容）；
> 3. 需要细节时追加对应专题章（如 P4 追加 06、07 章）；
> 4. 交付后用 [12-testing-and-acceptance.md](12-testing-and-acceptance.md) 对应阶段勾选表逐条核对。
>
> **契约版本**：对齐 **C 章 v1.4**（2026-11-06）。命令/事件/错误码以 C.6/C.7/C.8 为唯一来源，
> 本章各卡末尾的契约对齐记录见文末。
>
> **纪律**：一次只做一张卡。禁止跳卡、禁止合并卡、禁止「顺手把下一阶段也做了」。

---

## 13.0 总览

| 卡 | 阶段 | 主题 | 前置 | 工时 | 主要产出 | 关键门禁 |
| --- | --- | --- | --- | --- | --- | --- |
| P0 | 脚手架 | 可编译可运行的空壳 | — | 0.5 天 | 目录骨架、单实例、日志、托盘、构建脚本 | 启动 ≤ 1.2s、门禁全绿 |
| P1 | 内核 | mihomo sidecar + 系统代理 | P0 | 2.5 天 | 内核起停、端口分配、secret、WinINET 代理 | M1 ≤ 90MB、M6 ≤ 3s |
| P2 | 订阅 | 拉取/解析/合并/流量 | P1 | 3 天 | 订阅 CRUD、UserInfo、runtime config | 7 场景 Mock 全过 |
| P3 | 调度 | 单一 tick 调度器 | P2 | 1.5 天 | due/backoff/runner、自动更新 | M3、M9、15min 实测 |
| P4 | 测速 | 并发测速 + 自动切换 | P3 | 3 天 | engine/probe/rank/selector | M4、防抖三条件 |
| P5 | 界面 | 三页面 UI + 托盘完善 | P4 | 3 天 | 页面、空错态、虚拟滚动、托盘菜单 | M1、M2 |
| P6 | 轻量 | 性能调优到达标 | P5 | 1.5 天 | release profile、降采样、流式写 | C.11 九项全达标 |
| P7 | 打包 | NSIS + 合规声明 | P6 | 1.5 天 | 安装包、About、图标、CI | M7 ≤ 12MB、M8 |
| P8 | 验收 | 全量回归 + 收尾 | P7 | 2 天 | 20 条异常通过、基线、Release Notes | 全部验收表 |
| P9 | 运维 | 交付后的契约与性能账本 | P8（可与 P7/P8 并行，验收前完成） | 0.5 天 | `perf-log.md`、`check-contract.ps1` 入 CI、契约变更流程 | 契约不自漂 |

**关键路径**：P0 → P1 → P2 → P3 → P4 → P5 → P6 → P7 → P8 → P9，**P0–P8 严格串行**。
P9 是**必做**的第 10 张卡：可与 P7/P8 并行开工，但**必须在验收收尾（P8）结束前完成**。
理由：没有「契约对账 + 性能台账 + 变更流程」，下游一旦并行开发就必然漂移——
本次多 Agent 协作已实际踩到这个坑（契约被修了 3 轮）。P9 就是把这个坑堵上。
其中 P1 与 P2 之间有一处可并行的小缝隙（P2 的 `userinfo.rs` + `parse.rs` 纯函数可在 P1 期间并行开发），
但**合并成 runtime config 必须等 P1 的端口/secret 逻辑可用**。

**每张卡开工前必须做的三件事**（写进指令块，此处统一说明）：
1. 输出「改动文件清单 + 关键类型/函数签名 + 风险点 + 验证方式」四件套，等确认；
2. 声明本卡会用到 / 新增的 IPC 命令、事件、错误码，且必须全部已存在于 C.6/C.7/C.8；
3. 声明本卡完成后要跑的验收表小节（12.7 的哪一节）。

---

## 13.1 P0 · 脚手架

### 目标
建出「能编译、能启动、能进托盘、能写日志、单实例生效」的空壳。**不含任何业务逻辑**。

### 前置依赖
无。只需 Rust stable、Node 20+、pnpm、Tauri CLI、WebView2。

### 改动文件清单（路径以 C.2 为准）

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/Cargo.toml` | 新建 | 依赖白名单见 10.4.11；release profile 见 10.4.1 |
| `src-tauri/tauri.conf.json` | 新建 | identifier `com.clashzhengl.app`、productName `Clash Zhengl`、`devtools: false` |
| `src-tauri/capabilities/default.json` | 新建 | 最小权限，只开需要的 |
| `src-tauri/icons/` | 新建 | 16/24/32/48/64/128/256 + `icon.ico`（10.4.7） |
| `src-tauri/src/main.rs` | 新建 | 日志初始化 → panic hook → 单实例 → tray → 窗口 |
| `src-tauri/src/app.rs` | 新建 | `AppState` 骨架（字段先占位，P1/P2 填）、`ready`/`exit` 生命周期 |
| `src-tauri/src/domain/mod.rs` / `model.rs` / `errors.rs` | 新建 | `AppError` 按 C.8 定义（先只有 code/message/detail/retryable） |
| `src-tauri/src/commands/mod.rs` + `dto.rs` | 新建 | 只注册一个 `app_bootstrap`（返回空结构） |
| `src-tauri/src/system/paths.rs` | 新建 | Roaming/Local 两类目录，按 C.3 |
| `src-tauri/src/system/single_instance.rs` | 新建 | 命名互斥 + 唤起既有窗口 |
| `src-tauri/src/system/tray.rs` | 新建 | 最小托盘：显示/退出 |
| `src-tauri/src/util/log.rs` | 新建 | tracing + 滚动 + 脱敏层占位 |
| `src/main.ts` / `src/ipc/client.ts` / `src/ipc/events.ts` | 新建 | invoke 封装 + 错误归一 |
| `src/i18n/zh-CN.json` | 新建 | 键值化文案骨架 |
| `scripts/dev.ps1` / `scripts/build.ps1` | 新建 | 开发与构建入口 |
| `scripts/perf-probe.ps1` | 新建 | 按 10.3.1 全文落地 |

### 交付物
- 可运行的 `clash-zhengl.exe`（`cargo tauri build --no-bundle` 产出）
- 托盘图标出现，右键有「显示主窗口 / 退出」
- 关闭窗口进托盘（此时可不实现设置项，先固定行为）
- `%LOCALAPPDATA%\ClashZhengl\logs\app-YYYYMMDD.log` 有内容且格式符合 C.9
- `scripts/perf-probe.ps1` 可跑 `-Mode startup`

### 验收标准
- [ ] `cargo fmt --check` 无输出
- [ ] `cargo clippy --all-targets -- -D warnings` 无 warning
- [ ] `cargo test` 通过（至少 1 个 smoke：`AppError` 构造 + 序列化字段为 camelCase）
- [ ] `pnpm tsc --noEmit` 通过
- [ ] `cargo tauri build --no-bundle` 成功
- [ ] 冷启动到托盘可用 ≤ 1.2s（5 次中位数，release）
- [ ] 单实例：二次启动唤起已有窗口，`Get-Process clash-zhengl` 只有 1 个
- [ ] 日志首行含 RFC3339 时间 + LEVEL + `[module]`
- [ ] 故意 panic 一次：有日志、有中文提示、进程退出有记录
- [ ] `paths.rs` 输出路径与 C.3 完全一致（打印一次核对）

### 预估工时
0.5 天（4 小时）。其中环境搭建 1h、骨架 2h、脚本 1h。

### 执行指令块（可直接复制给 AI Agent）

```text
你是资深 Windows 桌面客户端工程师。本阶段是 P0 脚手架，目标是建出「能编译、能启动、能进托盘、能写日志、单实例生效」的空壳，不写任何业务逻辑。

【必读上下文】
- 已投喂的 01-master-prompt.md 是总纲与铁律；C-conventions.md 是唯一契约，冲突时以 C 为准。
- 严格按 C.2 的目录结构建文件，路径不得自创。严格按 C.1 的产品标识（com.clashzhengl.app / clash-zhengl.exe / Clash Zhengl）。
- 严格按 C.3 的运行时路径（Roaming=%APPDATA%\ClashZhengl，Local=%LOCALAPPDATA%\ClashZhengl）。
- 严格按 C.9 的日志单行格式：<RFC3339 本地时间> <LEVEL> [<module>] <message> key=value。

【技术约束】
- Rust stable + Tauri 2；前端 TypeScript + Vite，原生 DOM，禁止 React/Vue/任何 UI 框架。
- tokio 用 current_thread，禁止 #[tokio::main] 默认多线程；max_blocking_threads 收紧到 4。
- release profile 按 10 章 10.4.1 配置（opt-level="z"、lto="fat"、codegen-units=1、strip=true、incremental=false）；本阶段 panic 保持默认 unwind。
- 前端零重依赖：不引入 lodash/moment/字体包；日期用 Intl.DateTimeFormat。
- 依赖只允许：tauri、tokio、serde、serde_json、tracing、tracing-subscriber、tracing-appender、chrono、rand、thiserror、windows（按需 feature）。其它依赖需先说明理由与体积影响。

【本阶段要交付的文件】
1. src-tauri/Cargo.toml、tauri.conf.json、capabilities/default.json、icons/
2. src-tauri/src/main.rs（日志→panic hook→单实例→tray→窗口，顺序不可乱）
3. src-tauri/src/app.rs（AppState 骨架，只放 core 状态占位与 started_at）
4. src-tauri/src/domain/{mod,model,errors}.rs（AppError：code/message/detail/retryable；错误码枚举先只放 C.8 全部 17 个，不许多不许少）
5. src-tauri/src/commands/{mod,dto}.rs（只注册 app_bootstrap，返回 {settings:null, subscriptions:[], runtime:null} 占位）
6. src-tauri/src/system/{paths,single_instance,tray}.rs
7. src-tauri/src/util/log.rs（滚动日志 + 单文件 2MB 守卫 + non-blocking + lossy(true)）
8. src/main.ts、src/ipc/client.ts（invoke 封装 + AppError 归一）、src/ipc/events.ts
9. src/i18n/zh-CN.json
10. scripts/dev.ps1、scripts/build.ps1、scripts/perf-probe.ps1（perf-probe 按 10 章 10.3.1 全文实现，pwsh 7）

【执行规则】
1. 先输出「改动文件清单 + 关键类型与函数签名 + 风险点 + 验证方式」四件套，等我确认后再落盘。
2. 代码必须完整文件内容，不要省略号，不要「其余不变」。
3. 生产路径禁止 unwrap()/expect()（启动期不可恢复错误可用 expect 且必须带中文文案）。
4. 若发现契约未覆盖处，输出 CONTRACT-CHANGE-REQUEST: 说明，不要自行发明命令名或错误码。

【完成判定（你必须自己跑并在答复里贴出结果）】
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
pnpm tsc --noEmit
cargo tauri build --no-bundle
pwsh -File .\scripts\perf-probe.ps1 -Mode startup -Runs 5

【交付答复必须包含】
本阶段完成项 / 未完成项 / 已知问题 / 下一步建议 / 需要用户确认的问题；
并贴出 perf-probe 的启动耗时数字（中位数 ≤ 1200ms 才算通过）。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | `#[tokio::main]` 默认多线程 | RSS 凭空多 8 MB，M1 后续难达标 | 手写 `Builder::new_current_thread()` |
| 2 | release profile 写在 workspace 根 Cargo.toml | 不生效，包体 30 MB+ | 必须在 `src-tauri/Cargo.toml` 里写 |
| 3 | 托盘图标用主图标缩放 | 托盘模糊、启动慢 | 单独 16/32 px 图标内嵌 |
| 4 | 日志每行 flush | 启动时 IO 抖动 | non-blocking + `lossy(true)` |
| 5 | 单实例用文件锁 | 异常退出后锁残留，再也启动不了 | 用命名 mutex（`CreateMutexW`），进程退出自动释放 |
| 6 | panic hook 里再 panic | 死循环 | hook 里只做 `write!` 到 stderr，不分配不锁 |
| 7 | 前端引入 framework | 后续无法回头 | 从第一天就原生 DOM |
| 8 | `app_bootstrap` 返回 null 但 TS 类型写了具体结构 | 运行时 `Cannot read property` | 占位结构与类型保持一致 |

---

## 13.2 P1 · 内核与系统代理

### 目标
把 mihomo 作为独立 sidecar 进程管起来：能下载/校验/解压、能起停、能健康检查、能崩溃重启；
系统代理能一键开关且异常后可恢复。

### 前置依赖
P0 完成（AppState、日志、错误码、路径全部可用）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/src/kernel/mod.rs` | 新建 | 模块导出 |
| `src-tauri/src/kernel/supervisor.rs` | 新建 | 进程生命周期、健康检查（A.8 流程）、崩溃重启退避 |
| `src-tauri/src/kernel/api.rs` | 新建 | REST 客户端（`no_proxy()`、Bearer、超时、状态码映射） |
| `src-tauri/src/kernel/ports.rs` | 新建 | `TcpListener` 探测 + 重试 3 次 |
| `src-tauri/src/kernel/fetch_core.rs` | 新建 | v1.19.33 资产选择（A.7）、下载、SHA256、解压、版本目录管理 |
| `src-tauri/src/system/sysproxy.rs` | 新建 | WinINET 注册表 + `InternetSetOption` 刷新 + 恢复守卫 |
| `src-tauri/src/system/autostart.rs` | 新建 | 注册表 Run 键 |
| `src-tauri/src/system/notify.rs` | 新建 | 通知 + 托盘气泡降级 |
| `src-tauri/src/commands/core.rs` | 新建 | `core_status/core_start/core_stop/core_restart/core_install/core_versions_list/sysproxy_set` |
| `src-tauri/src/app.rs` | 修改 | 组装 kernel 状态，退出时按「关代理→停内核→退进程」顺序 |
| `src-tauri/tauri.conf.json` | 修改 | 确认无 sidecar 捆绑（策略 a：首启下载） |
| `scripts/fetch-mihomo.ps1` | 新建 | 开发期手动下载内核（支持 `--proxy`） |

### 交付物
- 点「下载内核」→ 落到 `runtime/core/v1.19.33/mihomo.exe`，About 显示版本；下载中发 `core:install-progress`（四阶段、≤2 条/秒）
- 设置页可从 `core_versions_list` 拿到 `installed/current/available` 三组数据，离线时 `available` 为空数组且不报错
- 点「启动内核」→ `GET /version` 返回 `v1.19.33`，`GET /configs` 的 `mixed-port` 与期望一致
- 系统代理开关生效（浏览器可访问 `http://www.gstatic.com/generate_204`），`sysproxy_set` 返回 `SysProxyState`
- 系统代理状态变化（含被外力改动）发 `sysproxy:state`
- 杀内核 → 自动重启（退避 1s/2s/4s，上限 3 次后置 Failed）
- 强杀应用 → 重启后系统代理未残留（`runtime/sysproxy.journal.json` 守卫）

### 验收标准
- [ ] `runtime/secret` 为 32 hex，文件 ACL 仅当前用户
- [ ] `external-controller` 只监听 127.0.0.1（`netstat -ano | findstr :<port>` 核对）
- [ ] 内核配置含 `-d` 与 `-f` 绝对路径，不含 `-secret` / `-ext-ctl`（A.11）
- [ ] `-t` 配置自检返回 0
- [ ] A.8 健康检查流程 4 个分支全部实现且有日志
- [ ] `core_*` 六命令幂等（含 `core_versions_list`；连点 3 次不产生多进程）
- [ ] `core:install-progress` 阶段齐全（`download→verify→extract→done`）、≤2 条/秒、percent 单调（异常清单 #22）
- [ ] 断点续传：`runtime/download/*.part` + `.part.json` 生效，完成后清理（异常清单 #21、C.3 v1.1）
- [ ] 端口占用场景：换端口成功（异常清单 #12）
- [ ] secret 不一致：返回 `E_CORE_API_UNAUTH` 并能自愈（异常清单 #13）
- [ ] 系统代理开启后 `curl -x http://127.0.0.1:<mixed-port> https://www.gstatic.com/generate_204` 返回 204
- [ ] 系统代理关闭后注册表 `ProxyEnable=0`，浏览器直连
- [ ] `RuntimeState.sysproxy` 四字段（`enabled/server/managed_by_us/port`）语义正确（C.5 v1.1）
- [ ] 外力改动注册表 → 发 `sysproxy:state`，UI 自动同步（异常清单 #23）
- [ ] 强杀后重启检测残留并恢复（异常清单 #18）
- [ ] 性能：M1 ≤ 90 MB、M6 ≤ 3s

### 预估工时
2.5 天。内核管理 1.5 天，系统代理 1 天。

### 执行指令块

```text
你是资深 Windows 桌面客户端工程师。本阶段是 P1 内核与系统代理。目标：把 mihomo v1.19.33 作为独立 sidecar 进程稳定管起来，并实现 Windows 系统代理一键开关与异常恢复。

【必读上下文】
- C-conventions.md 是唯一契约（当前 v1.1）。内核路径按 C.3（runtime/core/<version>/、runtime/download/ 为. part 续传区），secret 按 C.12（127.0.0.1 + 32 hex 随机，每次启动重新生成）。
- C.5 v1.1 新增 SysProxyState：{enabled, server, managed_by_us, port}；RuntimeState.sysproxy 字段是 UI 顶栏/托盘开关的初始化依据。
  managed_by_us 必须区分「我们写入的」与「用户原本的」，否则崩溃恢复会误关用户自己配的代理。
- 附录 A（appendices/A-mihomo-api-and-config.md）是内核 API 与配置字段的唯一依据：A.2 端点表、A.4 延迟语义、A.6 配置字段、A.7 资产选择、A.8 健康检查流程、A.9 常见坑、A.11 CLI 参数。
- 错误码只能用 C.8 中已有的：E_CORE_NOT_FOUND / E_CORE_START_FAILED / E_CORE_API_UNAUTH / E_CORE_PORT_EXHAUSTED / E_NET_* / E_STORE_IO / E_PERM_ADMIN / E_UNSUPPORTED / E_BUSY。
- 事件只能用 C.7 v1.1 的 core:state / core:install-progress / sysproxy:state / notify，不许自创。

【技术约束】
- 内核下载按 A.7：默认选 mihomo-windows-amd64-compatible-v1.19.33.zip；按 std::env::consts::ARCH 分支；拿不到校验值要记 WARN 并明说「未校验」，不许假装校验过。
- 下载到 runtime/download/<name>.part + <name>.part.json（续传偏移），完成后校验 SHA256 再解压到 runtime/core/<version>/，最后清理临时区（C.3 v1.1）。
- 下载/校验/解压全过程发 core:install-progress：{version, phase, received, total, percent}，
  phase ∈ download|verify|extract|done；合并到 ≤2 条/秒（C.7 v1.1），percent 单调不减。
- core_versions_list 返回 {installed, current, available}；available 来自 GitHub Release，
  **需网络，失败返回空数组且不报错**（C.6 v1.1），不许弹错误对话框。
- 只用 CLI：-d <runtime dir>、-f <config.yaml>、-t（自检）、-v（诊断）。禁止 -secret 与 -ext-ctl（会进进程列表，等于泄露 secret）。
- REST 客户端必须 .no_proxy()，否则回环请求会被用户自己的系统代理劫持；超时连接 1s / 读 3s。
- 端口分配：TcpListener::bind("127.0.0.1:0") 探测后释放，失败重试 3 次，全失败返回 E_CORE_PORT_EXHAUSTED。启动后必须 GET /configs 回读 mixed-port 核对。
- 子进程必须绑定 Windows Job Object（KILL_ON_JOB_CLOSE），保证应用被杀时内核不残留。
- 系统代理用 WinINET 注册表（HKCU\...\Internet Settings 的 ProxyEnable / ProxyServer / ProxyOverride）+ InternetSetOption 刷新。
  开启前把原始值快照写入 runtime/sysproxy.journal.json（C.3 v1.1），正常退出时写「已恢复」标记；
  下次启动若发现 journal 存在且未标记恢复 → 说明上次崩溃，须恢复原值。sysproxy_set 返回 SysProxyState，并在变化时 emit sysproxy:state。
- 默认非管理员运行；TUN 相关一律返回 E_PERM_ADMIN，本阶段不实现 TUN。

【本阶段要交付的文件（按 C.2）】
1. src-tauri/src/kernel/{mod,supervisor,api,ports,fetch_core}.rs（fetch_core 含 .part 续传与 SHA256）
2. src-tauri/src/system/{sysproxy,autostart,notify}.rs（sysproxy 含 journal 快照 + 外力改动探测）
3. src-tauri/src/commands/core.rs：core_status / core_start / core_stop / core_restart / core_install / core_versions_list / sysproxy_set（严格按 C.6 的入参与返回）
4. src-tauri/src/app.rs 修改：AppState 加入 core 状态机与 RuntimeState.sysproxy；退出顺序「关系统代理 → 停内核 → 退进程」
5. scripts/fetch-mihomo.ps1（开发期手动下载，支持 --proxy http://127.0.0.1:1886）

【执行规则】
1. 先输出四件套（改动文件清单 / 关键类型与函数签名 / 风险点 / 验证方式），确认后落盘。
2. 状态迁移必须覆盖 CoreState 全部 5 态（Stopped/Starting/Running/Stopping/Failed），并只在变化时 emit core:state（去重）。
3. 所有内核调用返回 Result<T, AppError>；detail 只放 status 与截断到 200 字符的 body；body 里的 secret/Bearer 必须脱敏。
4. journal 文件不得包含订阅 URL/token/节点密码；只放代理原始值与本应用端口。
5. 禁止 unwrap()/expect()；禁止裸 spawn 无回收；所有 async 有超时或取消路径。
6. 契约未覆盖处输出 CONTRACT-CHANGE-REQUEST:，不要自创命令名。

【完成判定（自己跑并贴结果）】
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cargo tauri build --no-bundle
pwsh -File .\scripts\perf-probe.ps1 -Mode idle -Seconds 60
pwsh -File .\scripts\perf-probe.ps1 -Mode startup -Runs 5
# 手工：下载内核（中途强杀验证续传）→ 开系统代理 → curl 验证 → 关代理 → 杀内核看自愈 → 强杀应用看代理残留
# 手工：离线状态下调 core_versions_list，确认 available 为空数组且无报错

【交付答复必须包含】
完成项/未完成项/已知问题/下一步建议/需确认问题；并贴 M1（RSS P95）与 M6（含内核代理可用）实测数字。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | reqwest 读了 `HTTP_PROXY` | 本地 API 全超时 | `.no_proxy()` |
| 2 | secret 拼进命令行 | 本机任意用户可读 | 只写配置文件（A.11） |
| 3 | 用 `-secret` 图方便 | 同上 | 同上 |
| 4 | 不做端口回读核对 | 内核悄悄换了端口，UI 状态与实际不符 | `GET /configs` 校验 |
| 5 | 子进程不绑 Job Object | 应用崩溃后 mihomo 残留，下次端口冲突 | Job Object + `KILL_ON_JOB_CLOSE` |
| 6 | 系统代理只在「正常退出」时恢复 | 强杀后用户断网 | 启动时检测残留（异常清单 #18） |
| 7 | `InternetSetOption` 忘了调 | 注册表改了但浏览器不生效 | 每次改完调 `INTERNET_OPTION_SETTINGS_CHANGED` + `REFRESH` |
| 8 | 下载 zip 解压全部文件 | 带进 README/LICENSE 到 runtime 目录，磁盘超预算 | 只取 `mihomo.exe` |
| 9 | 健康检查无上限轮询 | 内核起不来时永远 Starting | 200ms × 40 次 = 8s 上限（A.8） |
| 10 | 内核 stdout 直接继承父进程 | 日志混入应用日志、且可能打印 secret | 单独 `core-YYYYMMDD.log` + 过滤 secret |
| 11 | 下载进度每次都 emit | 20 MB 文件触发上千次事件，UI 卡死 | 500ms 合并，≤2 条/秒（C.7 v1.1） |
| 12 | `core_versions_list` 离线时报错 | 无网用户看到红色错误 | 失败返回 `available: []` + 不报错（C.6 v1.1） |
| 13 | 未清理 `runtime/download/` | 残留 `.part` 堆积，M8 磁盘超预算 | `done` 阶段后删除临时区 |
| 14 | journal 不写或写错 | 崩溃后误关用户自己的代理 | 只记录原始值 + `managed_by_us` 语义（C.5 v1.1） |

---

## 13.3 P2 · 订阅拉取与合并

### 目标
实现多订阅的拉取、解析、UserInfo 提取、合并成 `runtime/config.yaml`，并通过原子写落盘。
这是产品最核心的数据链路。

### 前置依赖
P1 完成（端口分配、secret 生成、配置文件路径可用）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/src/subscription/mod.rs` | 新建 | 导出 |
| `src-tauri/src/subscription/fetch.rs` | 新建 | UA、超时、代理、重试退避、ETag/If-None-Match、8 MB 上限 |
| `src-tauri/src/subscription/parse.rs` | 新建 | YAML 解析、类型统计、合法性校验、节点上限 |
| `src-tauri/src/subscription/userinfo.rs` | 新建 | **按附录 B.2 伪代码实现**，17 个单测 |
| `src-tauri/src/subscription/manager.rs` | 新建 | CRUD、去重、冲突前缀、合并生成 runtime config（流式写） |
| `src-tauri/src/store/json_store.rs` | 新建 | 原子写（tmp→fsync→rename）+ 损坏回退 |
| `src-tauri/src/store/crypto.rs` | 新建 | DPAPI 加解密 |
| `src-tauri/src/store/migrate.rs` | 新建 | schema_version 迁移骨架 |
| `src-tauri/src/commands/subscription.rs` | 新建 | 13 个 `subscription_*` + `traffic_summary` + `profile_preview` |
| `src-tauri/src/domain/model.rs` | 修改 | 落地 C.5 的 `Subscription`/`UserInfo`/`UpdateStatus`/`UserInfoSource` |
| `src-tauri/src/kernel/supervisor.rs` | 修改 | 配置重建后热重载或重启内核 |
| `tests/mock-sub-server.ps1` | 新建 | 按 12.3.1 全文 |
| `src-tauri/tests/fetch_scenarios.rs` | 新建 | 7 场景集成测试 |

### 交付物
- 可添加/编辑/删除/启用禁用多订阅，URL 落盘为 DPAPI 密文
- 每条订阅展示：节点数、分组名、流量（上传/下载/总量）、到期日
- 合并后 `runtime/config.yaml` 含 `mixed-port`/`external-controller`/`secret`/`proxies`/`proxy-groups`/`rules`（A.6.2 骨架）
- 7 个 Mock 场景行为符合 12.3.2 表

### 验收标准
- [ ] `userinfo.rs` 17 个单测全绿（附录 B.7）
- [ ] `/ok` → 四项有值；`/nouser` → 四项 `None` 且不报错；`/junk` → 部分 `None` 不崩
- [ ] `/500` → `E_HTTP_STATUS`；`/html` → `E_PARSE_YAML`；`/broken` → `E_NET_TIMEOUT`
- [ ] 失败时**保留旧 yaml**，只更新 `last_update_status` / `last_error`
- [ ] 原子写：强杀进程后无半截 JSON/YAML（`*.tmp` 残留可清理）
- [ ] 损坏文件重命名为 `*.corrupt-<ts>` 并回退默认值，UI 有提示
- [ ] 多订阅合并：节点名冲突加前缀，无重名，`proveder` 不出现（不用 provider）
- [ ] 禁用订阅不参与合并、不参与测速
- [ ] `proxies` 类型统计正确（Ss/Vmess/Vless/Trojan/Hysteria2/Tuic/Wireguard/Http/Socks5/Other）
- [ ] 8 MB 上限生效；5000 节点上限生效
- [ ] 合并后先跑 `mihomo -t` 自检，返回 0 才替换正式配置
- [ ] **生成的 `runtime/config.yaml` 中所有 `proxy-groups` 的 `type` 均为 `select`，无自动组**（附录 A.6.4：出现 `url-test`/`fallback`/`load-balance` 即失败）
- [ ] **关键取值正确**：`find-process-mode` 为 `strict`（**不是 `off`**）、`ipv6` 为 `true`、SystemProxy 档 `dns.enable` 为 `false`
- [ ] 订阅 URL 在日志/诊断包中无明文（`subscription_reveal_url` 除外）
- [ ] 合并 500 节点峰值内存增量 ≤ 50 MB

### 预估工时
3 天。fetch 0.8 天、parse 0.7 天、userinfo 0.5 天、manager/合并 1 天。

### 执行指令块

```text
你是资深 Rust 工程师，负责 Clash Zhengl 的 P2 阶段：订阅拉取、解析、UserInfo 提取、多订阅合并。

【必读上下文】
- C-conventions.md 是唯一契约。数据模型严格用 C.5 的 Subscription / UserInfo / UpdateStatus / UserInfoSource / ProxyKind，字段名与类型不得改。
- 附录 B（appendices/B-subscription-userinfo.md）是 subscription-userinfo 解析的唯一依据：B.2 解析算法、B.2.2 parse_expire 伪代码、B.3 total=0 语义、B.4 异常值表、B.7 的 15 个单测用例表。
- 附录 A 的 A.6 是 runtime/config.yaml 的字段依据，A.6.2 是最小骨架。
- 路径按 C.3：profiles/index.json、profiles/<uid>.yaml、profiles/<uid>.meta.json、runtime/config.yaml。
- 错误码只能用 C.8 已有项：E_NET_DNS / E_NET_TIMEOUT / E_NET_TLS / E_NET_PROXY / E_HTTP_STATUS / E_PARSE_YAML / E_PARSE_EMPTY / E_USERINFO_MISSING / E_STORE_IO / E_STORE_DECRYPT / E_BUSY。

【技术约束】
- 拉取：reqwest + rustls；UA 默认 "clash.meta/1.19.0"（settings.sub_user_agent）；超时 settings.sub_fetch_timeout_ms（默认 15000）；支持 sub_fetch_proxy；ETag/If-None-Match 命中 304 时不重写文件只更新 last_update_at。
- 重试：仅对 E_NET_TIMEOUT / E_NET_DNS / 5xx 重试，最多 2 次，退避 500ms/1500ms；4xx 不重试。
- 响应体上限 8 MB：优先看 Content-Length，无则边读边计数超限即中止。
- 落盘：全部 JSON 用 C.3 要求的原子写（写 *.tmp → fsync → rename）。失败绝不覆盖旧文件。
- 订阅 URL 与内核 secret 用 Windows DPAPI（CryptProtectData，CRYPTPROTECT_UI_FORBIDDEN）加密后落盘；内存中仅在发请求瞬间解密。
- UserInfo 解析：严格按附录 B.2 的四条铁律（split_once、逐字段独立失败、绝不 unwrap、raw 截断 512 字节）。total=0 必须保留为 Some(0)；expire=0 必须转为 None；13 位视为毫秒；越界 [2020,2100] 转 None。
- 合并生成 runtime/config.yaml 必须流式写（BufWriter 逐段 write!），禁止把全文读进 String 做替换；节点对象可用 serde_json::to_string 直接写（YAML 是 JSON 超集）。
- **配置取值硬约束（A.6.1 / A.6.3 / A.6.4，写错是静默失效，不会报错）**：
  · find-process-mode 必须写 strict。写 off 会让内核执行 helper.FindProcess = nil，
    此后机场下发的一切 PROCESS-NAME / PROCESS-PATH 规则永久失效，用户只会看到「某个软件连不上」。
  · ipv6 写 true（关掉会让纯 IPv6 站点与 IPv6 节点不可达，省的那点查询开销不值得）。
  · SystemProxy 档 dns.enable 写 false（updateDNS 在 !Enable 时直接 ReCreateServer("",nil,nil) 返回，
    根本不建服务；开了纯亏）。
  · proxy-groups 里**所有组的 type 都必须是 select**，禁止生成 url-test / fallback / load-balance
    （内核自带健康检查会与本应用测速抢当前节点）。在生成器出口写 assert_no_auto_groups()，命中即 Err。
- 禁止引入 provider 方案；禁止调用 subconverter（01 章 §2.2）。

【本阶段要交付的文件（按 C.2）】
1. src-tauri/src/subscription/{mod,fetch,parse,userinfo,manager}.rs
2. src-tauri/src/store/{json_store,crypto,migrate}.rs
3. src-tauri/src/commands/subscription.rs：subscription_list / subscription_add / subscription_update_url / subscription_rename / subscription_set_enabled / subscription_set_interval / subscription_remove / subscription_refresh / subscription_refresh_all / subscription_reveal_url / traffic_summary / profile_preview（严格按 C.6 的入参与返回）
4. src-tauri/src/domain/model.rs 落地 C.5 模型
5. tests/mock-sub-server.ps1（7 场景，pwsh 7，HttpListener 实现）
6. src-tauri/tests/fetch_scenarios.rs（7 场景集成测试 + common/mock_server.rs）

【事件（严格按 C.7，不许新增）】
- subscription:updated（负载 Subscription）
- subscription:progress（{uid, phase, percent?}，可选）

【执行规则】
1. 先输出四件套，确认后落盘。
2. 纯函数必须单测：userinfo 解析 17 例、parse 合法性、合并去重与冲突前缀、到期/剩余派生计算。
3. 生产路径禁止 unwrap()/expect()；错误统一 AppError，message 中文可读，detail 不得含 URL/token/密码。
4. 契约未覆盖的输出 CONTRACT-CHANGE-REQUEST:。

【完成判定（自己跑并贴结果）】
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
pwsh -File .\tests\mock-sub-server.ps1 -Port 18080   # 另一个终端
# 用 7 个路径逐一验证，贴出每个场景的实际错误码与落盘结果
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | 失败时先 `File::create` 再判断 | 旧配置被截断成 0 字节（BUG-0007） | 先写 tmp，成功后 rename |
| 2 | `total=0` 当成「未知」 | 不限量套餐显示成「—」 | 保留 `Some(0)`，UI 显示「不限量」 |
| 3 | `expire=0` 当成「1970 年到期」 | UI 红色「已过期」 | 转为 `None` |
| 4 | `expire` 毫秒没判别 | 显示 5138 年 | 13 位 ÷ 1000（B.2.2） |
| 5 | `split('=')` 而非 `split_once` | 畸形头 panic 或丢字段 | `split_once('=')` |
| 6 | 整头解析失败就整体 None | `total=abc` 把 upload 也丢了 | 逐字段独立失败 |
| 7 | 合并时 `String` 多次 clone | 峰值内存翻倍 | `&str` 切片 + 流式写 |
| 8 | 节点重名用「后覆盖前」 | 用户丢节点 | 加订阅前缀（`sub-a/HongKong-01`） |
| 9 | 直接覆盖 `runtime/config.yaml` | 内核正在读，读到半截 | 原子 rename + 重启/热重载 |
| 10 | 不跑 `-t` 自检就替换 | 内核起不来，用户断网 | 先 `-t`，返回 0 再替换 |
| 11 | HTML 错误页当 YAML 解析 | 报错信息不可读 | 先判 Content-Type，再判首字符 `<` |
| 12 | DPAPI 解密失败直接崩 | 换机器后应用不可用 | 返回 `E_STORE_DECRYPT` + UI 引导重填 |
| 13 | **把 `find-process-mode: off` 当「轻量优化」** | **机场的 `PROCESS-NAME` 规则静默失效**（mihomo `tunnel/tunnel.go` 对 `FindProcessOff` 执行 `helper.FindProcess = nil`），用户只看到「某个软件连不上」，日志里毫无线索 | 固定写 `strict`（内核默认值）；在生成器出口断言 `== "strict"` |
| 14 | **生成 `url-test` / `fallback` 组「让内核自己选更快」** | 内核健康检查与本应用测速形成两个决策者，节点来回跳，用户「选了没用」 | 只生成 `type: select`，自动切换完全由本应用显式 PUT 完成（A.6.4） |
| 15 | **`ipv6: false` 或 `dns.enable: true`** | 前者让纯 IPv6 站点/IPv6 节点不可达；后者在系统代理模式下白开一个 DNS 服务（`updateDNS` 在 `!Enable` 时根本不建服务，开了纯亏） | `ipv6: true`；SystemProxy 档 `dns.enable: false` |

---

## 13.4 P3 · 调度器

### 目标
实现单一 tick 调度线程，驱动「订阅定时更新」与「定时测速」两个周期任务，带抖动、退避、时间跳变处理。

### 前置依赖
P2 完成（订阅数据与刷新入口可用）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/src/scheduler/mod.rs` | 新建 | 导出 |
| `src-tauri/src/scheduler/clock.rs` | 新建 | 可注入时钟（`trait Clock { fn now(&self) -> i64 }`） |
| `src-tauri/src/scheduler/due.rs` | 新建 | **纯函数**：`(状态, now) -> Vec<DueTask>` |
| `src-tauri/src/scheduler/backoff.rs` | 新建 | 指数退避 + 抖动 |
| `src-tauri/src/scheduler/runner.rs` | 新建 | 单 tick 循环，对齐整分，串行执行 |
| `src-tauri/src/app.rs` | 修改 | 启动 runner，退出时优雅停止 |
| `src-tauri/src/commands/subscription.rs` | 修改 | `subscription_set_interval` 生效于调度器（下限 15 分钟） |
| `src-tauri/src/commands/settings.rs` | 新建 | `settings_get` / `settings_save`（hot-apply 调度参数） |

### 交付物
- 15 分钟周期真实观察：自动更新按周期发生，日志可证
- tick 间隔 ≥ 30s，空闲无 < 30s 唤醒
- 退避生效：连续失败 1s/2s/4s/8s/16s/30s（上限）

### 验收标准
- [ ] `due.rs` 9 个单测全绿（含时间回拨/前跳/禁用订阅/间隔夹紧）
- [ ] `backoff.rs` 5 个单测全绿（序列、抖动范围 0..30s、成功重置）
- [ ] 假时钟推进 24 小时，任务触发次数与理论值一致（±1）
- [ ] tick 对齐整分（日志时间戳秒数 ≈ 00）
- [ ] 启动抖动 0..30s 随机生效（多次启动时间不同）
- [ ] `subscription_set_interval` 小于 15 分钟被夹到 15 分钟
- [ ] 系统时间回拨 1 小时：不产生任务风暴，也不永久不触发
- [ ] 系统时间前跳 1 天：只补跑一次，不补跑 N 次
- [ ] 15 分钟实测：日志有 1 次自动更新记录
- [ ] 空闲 CPU ≤ 1.5%（M3）；10 分钟内 tick 次数 ≈ 10
- [ ] 无 `< 30s` 的后台唤醒（M9）

### 预估工时
1.5 天。

### 执行指令块

```text
你是资深 Rust 工程师，负责 Clash Zhengl 的 P3 阶段：单一 tick 调度器与自动更新。

【必读上下文】
- C-conventions.md 是唯一契约。时间约定按 C.10：落盘 Unix 秒 UTC；定时器精度 ±10%；所有周期任务启动加 0..30s 随机抖动；
  最短周期：订阅 15 分钟、测速 5 分钟。性能约束按 C.11 的 M3（空闲 CPU ≤1.5%）与 M9（空闲无 <30s 唤醒）。
- 10 章 10.4.10 是定时器合并与对齐的施工依据。
- 错误码用 C.8 已有项；事件只用 C.7 的 subscription:updated / subscription:progress / notify。

【技术约束】
- 只有一个 tick 循环（scheduler/runner.rs），间隔 30s 或 60s，对齐到整分；禁止 loop + sleep(1s) 轮询。
- due.rs 必须是纯函数：输入 (Vec<Subscription>, Vec<CycleState>, now) 输出到期任务集合；不碰 IO、不碰网络、不读系统时间（now 由参数传入）。
- clock.rs 提供可注入时钟 trait，单测用假时钟。
- backoff.rs：连续失败 1,2,4,8,16,30(上限) 秒；成功后重置；抖动 0..30s。抖动用 rand，但单测要把 rand 注入或提供纯函数计算区间。
- 时间跳变处理：间隔用单调时钟（std::time::Instant）计算，SystemTime 只用于算到期；睡眠唤醒后重新计算 next_due；回拨不重跑、前跳只补跑一次。
- 订阅更新串行且受 settings.sub_fetch_concurrency（默认 2，上限 4）约束，禁止同刻风暴。
- 内核未运行时：跳过测速任务并记 Info，不为了测速去拉起内核（除非 settings 明确要求）。
- 所有 async 任务必须有超时或取消路径；退出时 runner 要在 500ms 内响应停止信号。

【本阶段要交付的文件（按 C.2）】
1. src-tauri/src/scheduler/{mod,clock,due,backoff,runner}.rs
2. src-tauri/src/commands/settings.rs：settings_get / settings_save（严格按 C.6）
3. src-tauri/src/app.rs 修改：启动/优雅停止 runner
4. src-tauri/src/commands/subscription.rs 修改：interval 变更即时反映到调度器

【执行规则】
1. 先输出四件套（改动文件清单 / 函数签名 / 风险点 / 验证方式），确认后落盘。
2. due.rs 与 backoff.rs 的所有分支必须单测覆盖，含时间回拨、时间前跳、禁用订阅、间隔小于下限被夹紧。
3. 日志按 C.9：INFO [scheduler] tick tasks=N next_in=42s；WARN [scheduler] task backoff uid=xxx retry=1/3 wait=2s。
4. 禁止 unwrap()/expect()；禁止每秒轮询；禁止多个独立定时器。
5. 契约未覆盖的输出 CONTRACT-CHANGE-REQUEST:。

【完成判定（自己跑并贴结果）】
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
pwsh -File .\scripts\perf-probe.ps1 -Mode idle -Minutes 5
# 数 tick 次数：
$log = Get-ChildItem "$env:LOCALAPPDATA\ClashZhengl\logs\app-*.log" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
(Select-String -Path $log.FullName -Pattern '\[scheduler\] tick').Count
# 期望：10 分钟内约 10 次（tick=60s）；若上百次说明有轮询

【交付答复必须包含】
完成项/未完成项/已知问题/下一步建议/需确认问题；贴 M3（空闲 CPU）与 tick 次数实测值。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | `loop { sleep(1s); }` | M9 直接挂 | 单一 tick 对齐整分 |
| 2 | 每个订阅一个 `tokio::spawn` 定时器 | N 个定时器 N 次唤醒 | 统一 tick 派生 |
| 3 | 用 `SystemTime` 算间隔 | 用户改时间后任务风暴/永久停摆 | `Instant` 算间隔 |
| 4 | 时间前跳后补跑 N 次 | 一次累积触发几百次更新 | 只补跑一次（`last_run` 夹到 `now - interval`） |
| 5 | due.rs 里读系统时间 | 无法单测 | now 由参数传入 |
| 6 | 抖动用固定值 | 多实例/多任务同刻 | `rand 0..30s`，且每次启动重新取 |
| 7 | 退出时不等 runner | 托盘退出后还有后台任务写文件 | 500ms 内响应停止信号 |
| 8 | 更新失败不重置计时 | 一直失败一直重试 | 退避 + 上限 |
| 9 | 间隔下限没夹紧 | 用户设 1 分钟，机场封 IP | `max(15, interval)` |

---

## 13.5 P4 · 测速与自动切换

### 目标
基于 mihomo `/proxies/{name}/delay` 做并发测速，结果排序后按策略自动切换到最低延迟节点，带三重防抖。

### 前置依赖
P3 完成（调度器可触发测速）、P1 完成（内核 API 可用）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/src/speedtest/mod.rs` | 新建 | 导出 |
| `src-tauri/src/speedtest/engine.rs` | 新建 | 并发引擎（Semaphore）、聚合、事件合并 |
| `src-tauri/src/speedtest/probe.rs` | 新建 | 单节点探测（A.4.1）、失败重试一次 |
| `src-tauri/src/speedtest/rank.rs` | 新建 | **纯函数**排序（延迟升序、None 最后、排除规则、tolerance、**跳过锁定组**） |
| `src-tauri/src/speedtest/selector.rs` | 新建 | `PUT /proxies/{group} {"name":...}` + 防抖 + 幂等锁 + **`SwitchRuntimeLocks` 手动覆盖** |
| `src-tauri/src/domain/policy.rs` | 新建 | `SwitchPolicy` 判定纯函数（含 `manual_override_min`） |
| `src-tauri/src/commands/speedtest.rs` | 新建 | `speedtest_*`、`switch_apply_best`（**返回 `{primary, all}`**）、`switch_apply_node`、`switch_history` |
| `src-tauri/src/kernel/api.rs` | 修改 | **实现 `SelectorApi` trait**（`proxies_selectable()` / `proxy_group()` / `set_group_selected()`，**boxed-future `ApiFuture<'a,T>` 形式，不写裸 `async fn`**），产出 `GroupInfo` / `SelectableGroup`（字段映射见附录 A.2.3） |
| `src-tauri/src/scheduler/runner.rs` | 修改 | 接测速周期任务；**跳过处于手动覆盖期的组** |

### 交付物
- 300+ 节点并发测速 5s 内完成，进度事件 ≤ 4 条/秒
- 自动切换到最低延迟节点，`SwitchRecord` 含完整可解释字段
- 三重防抖生效：`tolerance_ms=30`、`min_dwell_sec=300`、`failure_threshold=2`
- **`switch_apply_best` 返回 `{primary, all}`；手动覆盖期内的组被跳过**
- **`kernel/api.rs` 的 `SelectorApi` 能列出所有组并区分 `Selector` / `URLTest`**（按 API 返回的 `type` 判定）

### 验收标准
- [ ] `rank.rs` 10 个单测全绿（含 None 排序、排除关键词、tolerance、地区优先、同延迟稳定排序、锁定组过滤）
- [ ] `selector.rs` 10 个单测全绿（min_dwell、failure_threshold、同名不重复 PUT、组不存在、**非 Selector 跳过**、**锁定中跳过**、**锁定到期恢复**、**再次手动选择续期**）
- [ ] 并发度实测不超过 `speedtest_concurrency`（抓内核日志/计时验证）
- [ ] `speedtest:progress` 实测 ≤ 4 条/秒
- [ ] `delay == 0` 视为失败（A.4.1）
- [ ] 单节点失败重试 1 次后计入 `error`
- [ ] 切换后 `GET /proxies/{group}` 的 `now` 与目标一致
- [ ] `switch:applied` 负载为单条 `SwitchRecord`，含 `fromNode/fromDelay/toNode/toDelay/reason`
- [ ] **`switch_apply_best` 返回 `Option<{primary, all}>`**：`scope=Group` 时 `all.len()==1`；`scope=Global` 3 组生效时 `all.len()==3`（12.6.3）
- [ ] **`primary == all[0]`，且 = 首个实际生效的组**（按实际生效顺序，不按 `group_names` 序、不按延迟序）
- [ ] **无任何组被切换时整体返回 `None`**（不是 `{primary: null, all: []}`）
- [ ] **`SwitchRecord.to_delay` 为 `Option<u32>`**：Manual 切换时为 `null` 而非 `0`（12.6.2 #28）
- [ ] **手动锁定后自动切换确实跳过该组**：`switch_apply_node` 后立即测速 → `switch_apply_best` 的 `all` 不含该组，且不产生该组 `switch:applied`（12.6.2 #27）
- [ ] `manual_override_min` 默认 60 分钟生效；=0 时手动选择可被立刻覆盖；到期后恢复参与排序
- [ ] `SwitchRuntimeLocks` **纯内存态**：不落盘、不进 `RuntimeState`、重启后清空
- [ ] **`proxies_selectable` 返回的组，其 `kind` 全部为 `"Selector"`（大驼峰），无一组为 `select`**
- [ ] **`kind_case_sensitive_*` 单测全绿**（12.4.2：`"Selector"`→选中 / `"select"`→不选中 / `URLTest`·`Fallback`·`LoadBalance`→不选中 / 无宽松比较）
- [ ] **`SelectorApi` / `ApiError` / `GroupInfo` / `SelectableGroup` 签名符合 C.6.1 冻结面**，且**未为其新增任何 IPC 命令**
- [ ] 目标组 `kind != "Selector"` 时**跳过并记 Warn**，不盲发 PUT（A.2.1；注意 API 返回是大驼峰 `Selector`，非 YAML 的 `select`）
- [ ] `switch_policy.scope` 指向的组在 `runtime/config.yaml` 中确为 `type: select`，无同名 `url-test`/`fallback`
- [ ] `speedtest_start` 幂等：重复调用返回同一 runId
- [ ] 全失败场景：不切换、有提示、不崩（异常清单 #19）
- [ ] 目标为 `DIRECT`/`REJECT` 等内置名：跳过 + Warn（A.9 坑 2）
- [ ] 测速期 CPU ≤ 25% 且 ≤ 5s（M4）
- [ ] 切换历史可解释：能回答「为什么切、从哪切到哪、当时延迟多少」（01 章 §3.7）

### 预估工时
3 天。engine/probe 1.2 天、rank/selector 1.2 天、命令与事件 0.6 天。

### 执行指令块

```text
你是资深 Rust 工程师，负责 Clash Zhengl 的 P4 阶段：并发延迟测速 + 自动切换到最低延迟节点 + 手动覆盖。

【必读上下文】
- C-conventions.md 是唯一契约（当前 v1.2）。测速数据结构严格按 C.5 的 SpeedtestRun / ProbeResult / ProxyKind / RunTrigger；
  切换策略严格按 C.5 的 SwitchPolicy：tolerance_ms 默认 30、min_dwell_sec 默认 300、failure_threshold 默认 2、
  manual_override_min 默认 60（0 = 不暂停）、exclude_keywords 默认 ["到期","过期","剩余","流量","官网","套餐","重置","客服","域名"]；
  切换记录按 SwitchRecord，注意 to_delay 是 Option<u32>（Manual 切换为 None，不得用 0 冒充）。
- C.5 v1.2「手动覆盖语义」：switch_apply_node 成功后该组在 manual_override_min 分钟内被自动切换跳过。
  覆盖状态是内存态 SwitchRuntimeLocks { group, locked_at }，放 speedtest/selector.rs 的运行态，
  不落盘、不进 RuntimeState、重启后清空；到期判定 now - locked_at >= manual_override_min*60；
  用户再次手动选同一组 → 以最后一次时间续期。
- C.5 v1.2「组类型约束」：switch_policy.scope 指向的组必须是 type: select。
  禁止同名 url-test/fallback（内核自带健康检查会与我们抢当前节点，表现为来回跳）。
- C.6 v1.2：switch_apply_best 返回 {primary: SwitchRecord, all: Vec<SwitchRecord>}，不再是单条！
  scope=Group 时 all 长度为 1；scope=Global 时为全部实际生效的组。
  switch_apply_node 返回单条 SwitchRecord。switch_history {limit?} 返回 Vec<SwitchRecord>（倒序，默认 50，上限 200）。
- 附录 A 的 A.4 是延迟端点语义唯一依据；A.2.1 是「PUT 只能用于 select 组」的依据；A.2.3 是 GroupInfo/SelectableGroup 的字段映射。
- A.9 坑 2：DIRECT/REJECT/REJECT-DROP/PASS/COMPATIBLE/GLOBAL 及其小写是受保护名，PUT 会 400。
- 10 章 10.4.3 是事件降采样的施工依据（speedtest:progress 批量合并 ≤4 条/秒）。
- 错误码只用 C.8 已有项；事件只用 C.7 的 speedtest:progress / speedtest:done / switch:applied / notify。

【技术约束】
- 探测：每个节点一次 GET /proxies/{name}/delay；并发受 tokio::Semaphore(speedtest_concurrency) 约束（默认 8，上限 16）；
  HTTP 客户端超时 = speedtest_timeout_ms + 1500；失败重试 1 次（仅超时/5xx）后记 error。
- 探测 URL 用 settings.speedtest_probe_url（默认 http://www.gstatic.com/generate_204），expected 默认 204。
- rank.rs 是纯函数：(Vec<ProbeResult>, &SwitchPolicy, &str current, &[SwitchRuntimeLocks]) -> RankOutcome；
  不碰网络、不读系统时间（now 由参数传入）。排序键：有延迟的按升序 → 无延迟的排最后 → 同延迟按 node_key 字典序（稳定）；
  应用 exclude_keywords、内置受保护名、以及处于锁定期的组过滤。
- selector.rs 应用切换：PUT /proxies/{group} body {"name":"<目标>"}；成功为 204（无响应体，不要当失败）。
  **PUT 之前必须先 GET /proxies/{group} 校验 kind == "Selector"**（A.2.1）；不是 Selector 则跳过并记 Warn，不得盲发。
  三重防抖：新节点须比当前快 ≥ tolerance_ms 才切；切换后 min_dwell_sec 内不再切；
  连续 failure_threshold 次探测失败才降级。切换必须幂等（同名不重复 PUT）。
  手动覆盖：switch_apply_node 成功后写 SwitchRuntimeLocks；switch_apply_best 跳过所有在锁定期的组。
- **头号约束（C.6.1，写错会静默失效）**：GroupInfo.kind 存的是内核 API 返回的 type，取值是
  **大驼峰 Selector / URLTest / Fallback / LoadBalance / Relay**；而订阅 YAML 里写的是小写连字符
  select / url-test / fallback / load-balance。二者不是同一套字符串。
  判定必须**大小写敏感精确匹配 kind == "Selector"**；
  **禁止** contains("select")、to_lowercase() == "select"、eq_ignore_ascii_case("select")
  （"Selector".to_lowercase() == "selector" ≠ "select"，而 "select".contains("select") 会误伤）。
  必须补一组 kind_case_sensitive_* 单测：传入 "Selector" → true；传入 "select" → false；URLTest/Fallback → false。
- switch_apply_best 实现要点（C.6 v1.3 定稿，不要自创规则）：
  返回 Option<{primary, all}>。遍历 scope 内各组 → 过滤锁定组与非 Selector 组 → 逐组判定是否需要切 →
  收集成功的 SwitchRecord 到 all（按**实际生效顺序**排列）→ primary = all[0]（首个实际生效的组，
  不按 group_names 顺序、不按延迟排序，且不得指向被抑制/未生效的组）。
  **无任何组被切换时整个返回值为 None**，不得返回 {primary: null, all: []}（契约明确不存在该形态）。
  primary 与 all[0] 恒等，测试必须断言这一点。
- 事件合并：用 tokio::time::interval(250ms) 攒批；每批只带 latest；finished/total 必须单调；单条负载 ≤ 8 KB。
- 取消：speedtest_cancel 只取消未开始的探测（已在飞的用 select! + CancellationToken 打断）。
- 禁止调用 DELETE /proxies/{name}、DELETE /connections、/providers/*/healthcheck（附录 A 禁用清单）。

【本阶段要交付的文件（按 C.2）】
1. src-tauri/src/speedtest/{mod,engine,probe,rank,selector}.rs（selector 含 SwitchRuntimeLocks 与 select 校验）
2. src-tauri/src/domain/policy.rs（SwitchPolicy 判定纯函数，含 manual_override_min 语义）
3. src-tauri/src/commands/speedtest.rs：speedtest_start / speedtest_status / speedtest_cancel /
   switch_apply_best（返回 Option<{primary, all}>）/ switch_apply_node / switch_history（严格按 C.6）
4. src-tauri/src/kernel/api.rs 修改：实现 SelectorApi trait，**必须用 boxed-future 形式**：
     pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;
     pub trait SelectorApi: Send + Sync {
         fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>>;
         fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>;
         fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()>;
     }
   **不要写裸 async fn**（Rust 1.85 下 async fn in trait 不 dyn 兼容，07 章无法注入 Arc<dyn SelectorApi> 的 FakeApi），
   **也不要用 async-trait 宏**（会扩大依赖白名单）。实现侧写 { Box::pin(async move { … }) }，调用侧照常 .await。
   产出 GroupInfo { name, kind, now, all, test_url, hidden } 与 SelectableGroup { name, members, now }
   （注意字段名是 members，不是 candidates；无 hidden 字段——隐藏组在 proxies_selectable 阶段即被过滤），
   字段映射严格按附录 A.2.3；这些都是内部类型，不是 IPC DTO，不新增命令。
5. src-tauri/src/scheduler/runner.rs 修改：接入测速周期任务（内核未运行则跳过并记 Info），并跳过锁定组

【执行规则】
1. 先输出四件套（改动文件清单 / 关键类型与函数签名 / 风险点 / 验证方式），确认后落盘。
   SelectorApi / ApiError / GroupInfo / SelectableGroup / SwitchRuntimeLocks 已纳入 C.6.1 内部接口冻结面：
   它们是 kernel/api.rs 与 speedtest/selector.rs 的内部类型，**不是 IPC 命令、不得为它们新增 invoke**；
   签名须与 C.6.1 一致，改动须同步 07 章 §7.16 冻结表。
2. rank.rs / policy.rs 全分支单测；用假 API（trait 抽象）测 engine 的并发与取消；
   selector.rs 的锁定跳过/到期/续期必须各有单测，且用注入的 now（不读系统时间）。
3. 每次切换必须写 SwitchRecord 并落日志（C.9）：
   INFO [speedtest] switch group=xxx from=A(120ms) to=B(45ms) reason=Best；
   Manual 切换记 INFO [speedtest] switch group=xxx to=B reason=Manual to_delay=none；
   跳过记 INFO [speedtest] skip group=xxx reason=locked|not-select remaining=42s。
4. 生产路径禁止 unwrap()/expect()；禁止裸 spawn 无回收；所有 async 有超时或取消路径。
5. 契约未覆盖的输出 CONTRACT-CHANGE-REQUEST:。SelectorApi 已在 C.6.1 冻结面内，
   若要增删其方法，须先同步 07 章 §7.16 冻结表并升冻结版本号，不得自行改签名。

【完成判定（自己跑并贴结果）】
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
pwsh -File .\scripts\perf-probe.ps1 -Mode speedtest
# 实测事件频率：开 Debug 日志后统计 1 秒内 speedtest:progress 条数，必须 <= 4
# 手工 A：300+ 节点跑一次，贴出 run 总耗时、best 节点、switch_apply_best 的 primary 与 all.len()
# 手工 B：手动选一个节点 → 立即测速 → 确认 all 不含该组、无该组 switch:applied
# 手工 C：把 manual_override_min 设为 0 → 重复 B，确认该组可被立刻覆盖
# 手工 D：switch_history 中 Manual 记录 toDelay 为 null（不是 0）

【交付答复必须包含】
完成项/未完成项/已知问题/下一步建议/需确认问题；贴 M4（测速期 CPU 峰值与持续时间）实测值。
```

### 常见坑

> **#1 是头号坑**：它不会报错、不会 panic、日志里也没有异常，只会让「自动切换到最低延迟节点」这个核心功能**静默失效**。
> 凡是「功能没反应但没报错」的现象，先查这一条。

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| **1** | **`kind` 用小写 `select` 比较**（订阅 YAML 写 `select`，但内核 API `/proxies` 返回 `Selector`） | **核心功能完全失效但不报错**：`proxies_selectable` 恒返回空 → `switch_apply_best` 永远返回 `None` → 用户看到「自动切换没反应」。`"Selector".to_lowercase()` = `"selector"` ≠ `"select"`，而 `"select".contains("select")` 又会误伤 | **只用大小写敏感精确匹配 `kind == "Selector"`**；禁止 `contains("select")` / `to_lowercase() == "select"`；必须写 12.4.2 的 `kind_case_sensitive_*` 单测 |
| 2 | 把 `/group/{name}/delay` 当主力 | 内核清空自动组固定选择 → 双切抖动 | 只用单节点端点（A.4.2） |
| 3 | `delay == 0` 当成功 | best 选中一个不可用节点 | 0 视为失败 |
| 4 | 节点名不编码 | 中文名 404 | `Url::path_segments_mut()` |
| 5 | 每次探测完成就 emit | 300 节点 = 300 次事件，UI 卡死 | 250ms 合并 |
| 6 | 204 当成失败 | `Invoke-RestMethod` 返回 null 被误判 | 用 `status.is_success()` |
| 7 | 不排除 `DIRECT` | 切到直连，用户以为节点失效 | 受保护名过滤 |
| 8 | 没有 min_dwell | 5 分钟内切十几次，用户感知断流 | `min_dwell_sec=300` |
| 9 | tolerance 太小或不设 | 45ms 与 46ms 反复互切 | `tolerance_ms=30` |
| 10 | 多个 selector 组不同步 | 切了「节点选择」但「自动选择」没变 | 按 `scope` 决定作用范围，并明确日志 |
| 11 | **`switch_apply_best` 还按单条 `SwitchRecord` 返回** | 前端 `res.group` 拿不到、`all` 未定义（v1.2 破坏性变更） | 返回 `Option<{primary, all}>`；前端同步改 |
| 12 | **`to_delay` 用 `u32` 或填 0** | Manual 切换在历史里显示「0ms」误导用户 | `Option<u32>`，Manual 时为 `None` |
| 13 | **忘了手动覆盖** | 用户手动选了节点，下次测速又被自动切走 → 「设置了没用」 | `SwitchRuntimeLocks` + `manual_override_min` |
| 14 | **锁定状态落盘了** | 重启后用户以为在自动模式，实际被旧锁定挡住 | 纯内存态，重启清空（C.5 v1.2） |
| 15 | **锁定状态塞进 `RuntimeState`** | 每 2s 的 `traffic:tick` 负载里夹带无关字段 | 放 `selector.rs` 运行态（C.5 v1.2 明确要求） |
| 16 | **对 `url-test` 组发 PUT** | 内核接受但下个 interval 又切回，表现为「假成功/来回跳」 | PUT 前校验 `kind == "Selector"`（A.2.1） |
| 17 | **合并配置时同名生成了 `url-test` 组** | 内核自带健康检查与本应用抢当前节点 | scope 组必须只生成 YAML 的 `type: select` |
| 18 | 测速时内核被重启 | 全部 404/连接拒绝 | 测速前检查 `CoreState::Running` |
| 19 | 并发度写成硬编码 8 | 用户改设置不生效 | 读 `settings.speedtest_concurrency` |
| 20 | 结果保留全历史 | 内存线性增长 | 只留最近 1 轮 + SwitchRecord ≤ 200 条 |

---

## 13.6 P5 · UI 与托盘

### 目标
三页面极简 UI（订阅 / 流量与到期 / 节点与测速 + 设置）+ 完整托盘菜单，原生 DOM，虚拟滚动，
事件驱动不重渲染。

### 前置依赖
P4 完成（所有后端能力可用）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src/main.ts` | 修改 | 路由、可见性降采样、全局错误处理 |
| `src/ipc/client.ts` | 修改 | 补齐 C.6 全部 **30** 个命令封装 + 错误归一 |
| `src/ipc/events.ts` | 修改 | 补齐 C.7 全部 **10** 个事件订阅 |
| `src/ui/subscriptions.ts` | 新建 | 订阅列表页 |
| `src/ui/traffic.ts` | 新建 | 流量卡 + 到期卡（C.5 派生规则） |
| `src/ui/nodes.ts` | 新建 | 节点列表 + 虚拟滚动 + 测速按钮 |
| `src/ui/settings.ts` | 新建 | 设置页 |
| `src/ui/components/*.ts` | 新建 | toast/confirm/progress/empty 等原子组件 |
| `src/ui/styles.css` | 新建 | 深浅色、DPI、无动画降级 |
| `src/i18n/zh-CN.json` | 修改 | 全部文案键值化 |
| `src-tauri/src/system/tray.rs` | 修改 | 完整托盘菜单 + 事件 |
| `src-tauri/src/commands/settings.rs` | 修改 | `logs_tail`、`diagnostics_export` |

### 交付物
- 三页面可切换，空态/加载态/错误态齐全
- 流量卡正确展示四种文案（有总量/不限量/未知/已超量）
- 节点数 > 300 可流畅滚动
- 托盘菜单：切换节点、开关代理、立即更新、立即测速、显示窗口、退出

### 验收标准
- [ ] 所有 C.6 命令在前端有封装，无契约外命令（12.4.1 契约测试通过）
- [ ] 所有 C.7 事件都订阅且正确解绑
- [ ] 流量卡四种文案正确（C.5 派生规则，含 `total=Some(0)` → 「不限量」）
- [ ] 到期卡：`expire=None` → 「无限期」；已过期 → 「已过期 N 天」红色
- [ ] 单位换算 1024 进制、2 位小数
- [ ] **切换历史里 `toDelay == null` 显示 `—`**，不显示 `0ms`（`toDelay=0` 也不能被当成 null，用 `=== null` 而非 `||`）
- [ ] **手动选择节点后 UI 明示「已手动锁定，N 分钟内不自动切换」**（取 `manual_override_min`，=0 时不显示）
- [ ] 节点 > 300 时虚拟滚动或分页，滚动不掉帧
- [ ] 事件风暴（连续测速）下不重建列表、不闪烁
- [ ] 窗口隐藏时 traffic 推送降频/暂停（10.4.4）
- [ ] 托盘菜单 6 项全部可用；双击托盘恢复窗口
- [ ] 关闭窗口进托盘（`close_to_tray`）
- [ ] 深浅色跟随系统；DPI 125%/150% 不糊
- [ ] 中文文案无英文残留，无 lorem
- [ ] 性能：M1 ≤ 90 MB、M2 ≤ 140 MB

### 预估工时
3 天。订阅页 0.7、流量页 0.6、节点页 1、设置页 0.4、托盘 0.3。

### 执行指令块

```text
你是资深前端工程师（原生 DOM + TypeScript，无框架），负责 Clash Zhengl 的 P5 阶段：三页面 UI 与托盘。

【必读上下文】
- C-conventions.md 是唯一契约（当前 v1.1）。IPC 命令只能用 C.6 表里的 30 个，事件只能用 C.7 表里的 10 个，错误码只用 C.8 的 17 个。
  特别注意：C.6 明确「不做 `ui_visibility_set`」——窗口可见性由 Rust 侧监听 Tauri `WindowEvent` 自行判断，前端不得为此新增 IPC。
  命令入参与返回统一 camelCase（C.4）。
- C.5 的「派生字段规则」是前端展示的唯一依据：已用=upload+download（任一缺失显示 —）；剩余=total-已用（total==0 或缺失显示「不限量」/—）；
  使用率在 total==0 时不计算；剩余天数=expire_at-now（None 显示「无限期」）；单位 1024 进制 2 位小数。
  前端只展示不重算，重算逻辑必须来自后端。
- C.7 事件限流：traffic:tick 运行期每 2s，窗口不可见降到 10s 或暂停；speedtest:progress ≤ 4 条/秒。
- 10 章 10.4.3 / 10.4.4 是事件降采样与 webview 隐藏暂停的施工依据。
- 09 章是 UI/UX 细节依据（信息架构、组件树、空错态、虚拟滚动、托盘结构）。

【技术约束】
- 原生 DOM + TypeScript + Vite。禁止 React/Vue/Svelte/antd/任何 UI 框架；禁止 lodash/moment；日期用 Intl.DateTimeFormat。
- 不打包中文字体，用系统字体栈（"Microsoft YaHei UI", "Segoe UI", system-ui）。
- 收到事件只更新受影响节点的 textContent，禁止 innerHTML 重建列表；节点 > 300 用虚拟滚动（只渲染视口 ~40 行）。
- 所有用户可见文案走 i18n（zh-CN.json），不硬编码中文到 DOM 字符串里（模板里取键）。
- 错误统一经 src/ipc/client.ts 归一为 {code, message, detail?, retryable}，UI 只显示 message，detail 折叠在「详情」里。
- 任何按钮点击后进入 loading 态并防重复点击；危险操作（删除订阅、退出）二次确认。

【本阶段要交付的文件（按 C.2）】
1. src/main.ts（路由、可见性监听、全局未捕获错误提示）
2. src/ipc/client.ts（补齐 C.6 全部命令封装 + 错误归一）
3. src/ipc/events.ts（补齐 C.7 全部事件订阅，含解绑）
4. src/ui/subscriptions.ts / traffic.ts / nodes.ts / settings.ts
5. src/ui/components/*.ts（toast / confirm / progress / empty / skeleton）
6. src/ui/styles.css（深浅色、DPI 适配、body.is-hidden 时 animation/transition 全关）
7. src/i18n/zh-CN.json
8. src-tauri/src/system/tray.rs 修改（完整托盘菜单 + 事件）
9. src-tauri/src/commands/settings.rs 修改（logs_tail / diagnostics_export，严格按 C.6）

【执行规则】
1. 先输出四件套（改动文件清单 / 组件树与状态流 / 风险点 / 验证方式），确认后落盘。
2. 先给出文字版控件树与状态流转（01 章 §6 要求），再写代码。
3. 三页面的空态/加载态/错误态必须逐一实现，不许只做正常态。
4. 托盘菜单构建用 Tauri 2 API，菜单项变更需重建；不要在托盘回调里做重活（转发到 async 任务）。
5. 契约未覆盖的输出 CONTRACT-CHANGE-REQUEST:，不要自创命令或事件名。

【完成判定（自己跑并贴结果）】
pnpm tsc --noEmit
pnpm biome check
pnpm build
cargo tauri build --no-bundle
pwsh -File .\scripts\perf-probe.ps1 -Mode idle -Seconds 60
pwsh -File .\scripts\perf-probe.ps1 -Mode visible -Seconds 60
# 手工：造 300+ 节点订阅，滚动列表；连续点 3 次测速看界面是否闪烁；切深浅色；DPI 150%

【交付答复必须包含】
完成项/未完成项/已知问题/下一步建议/需确认问题；贴 M1（隐藏态）与 M2（可见态）RSS 实测值。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | 事件回调里 `innerHTML = ...` | 每次 tick 重建 300 行，UI 卡 | 只改 textContent |
| 2 | 不解除事件监听 | 切页面后重复响应，内存增长 | `listen()` 返回的 unlisten 必须调用 |
| 3 | 前端自己算「剩余流量」 | 与后端口径不一致 | 只展示后端派生值 |
| 4 | 引入 UI 框架 | 包体 +200KB、RSS +20MB | 原生 DOM |
| 5 | 打包中文字体 | 包体 +6MB | 系统字体栈 |
| 6 | 隐藏时不降频 | M3 直接超 1.5% | visibilitychange 通知后端 |
| 7 | 托盘回调里做网络请求 | 托盘菜单卡住、点不动 | 转发到 async 任务 |
| 8 | 删除订阅无二次确认 | 误删用户配置 | confirm 组件 |
| 9 | 错误 detail 直接显示 | 可能泄露 URL/token | 只显示 message，detail 折叠且脱敏 |
| 10 | DPI 用固定 px | 150% 下糊 | 用 rem/相对单位 + CSS 变量 |

---

## 13.7 P6 · 轻量化调优

### 目标
把 C.11 的九项性能指标全部压到达标，并把实测数字记录成基线。

### 前置依赖
P5 完成（功能齐全，才有完整的内存/CPU 画像）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/Cargo.toml` | 修改 | release profile（10.4.1）、依赖 feature 裁剪（10.4.11） |
| `src-tauri/src/main.rs` | 修改 | tokio `current_thread` + `max_blocking_threads(4)` |
| `src-tauri/src/util/log.rs` | 修改 | non-blocking + `lossy(true)` + 2MB 大小守卫（10.4.6） |
| `src-tauri/src/subscription/manager.rs` | 修改 | 流式写 runtime config（10.4.5） |
| `src-tauri/src/speedtest/engine.rs` | 修改 | 事件合并窗口（10.4.3） |
| `src-tauri/src/app.rs` | 修改 | 窗口可见性 → traffic 推送频率（10.4.4）、按需启动内核（10.4.9） |
| `src/main.ts` / `src/ui/styles.css` | 修改 | 隐藏暂停渲染、去动画 |
| `src-tauri/tauri.conf.json` | 修改 | `devtools: false`、`resources: []` |
| `src-tauri/icons/*` | 修改 | 图标裁剪（10.4.7） |
| `docs/perf-baseline.md` | 新建 | 按 10.6 模板 |

### 交付物
- `perf-probe.ps1 -Mode all` 退出码 0
- `docs/perf-baseline.md` 含十项实测数字
- 若有不达标项：`docs/perf-exception.md` 按 10.7 说明

### 验收标准
- [ ] M1 主进程 RSS（隐藏+内核）≤ 90 MB
- [ ] M2 主进程 RSS（可见）≤ 140 MB
- [ ] M3 空闲 CPU ≤ 1.5%
- [ ] M4 测速期 CPU ≤ 25% 且 ≤ 5s
- [ ] M5 冷启动到托盘 ≤ 1.2s
- [ ] M6 含内核代理可用 ≤ 3s
- [ ] M7 安装包 ≤ 12 MB
- [ ] M8 常驻磁盘 ≤ 120 MB
- [ ] M9 无 < 30s 后台唤醒
- [ ] release profile 已配置且 `panic` 取值有结论
- [ ] 无 React/Vue/字体包/大图（`cargo bloat` + `dist` 体积核对）
- [ ] `docs/perf-baseline.md` 已填写（含内核内存参考值）
- [ ] 10.8 反模式黑名单 15 条逐条自查无命中

### 预估工时
1.5 天（含 3 轮测量）。

### 执行指令块

```text
你是资深 Rust/性能工程师，负责 Clash Zhengl 的 P6 阶段：轻量化调优，把 C.11 九项性能指标压到达标。

【必读上下文】
- C.11 是硬指标，数字不得修改：RSS 隐藏 ≤90MB、可见 ≤140MB、空闲 CPU ≤1.5%、测速期 CPU ≤25% 且 ≤5s、
  冷启动到托盘 ≤1.2s、含内核代理可用 ≤3s、安装包 ≤12MB、常驻磁盘 ≤120MB、空闲无 <30s 唤醒。
- 10 章是施工手册：10.3.1 是 perf-probe.ps1 全文，10.4 是逐条优化手段（构建期 profile / tokio current_thread /
  事件降采样 / webview 隐藏暂停 / YAML 不常驻 / 日志缓冲 / 图标裁剪 / NSIS 压缩 / 按需启动内核 / 定时器合并 / 依赖白名单），
  10.7 是 L0–L4 降级阶梯（必须按顺序，不许跳级），10.8 是反模式黑名单，10.6 是基线记录模板。
- 任何降级都要在 PR 与 docs 里写明「降了哪一级、为什么、影响什么」。

【技术约束与手段（按 10.4 逐条落实）】
1. src-tauri/Cargo.toml 的 [profile.release]：opt-level="z"、lto="fat"、codegen-units=1、strip=true、incremental=false。
   panic 的取舍：只有同时满足「clippy 全绿且生产路径无 unwrap/expect」「不依赖 catch_unwind」「有崩溃自动重启看护」三条才可设 abort，
   否则保持 unwind 并写明理由。
2. tokio 必须 Builder::new_current_thread().enable_all().max_blocking_threads(4)；async 路径禁止 std::fs 大文件读。
3. 事件降采样：traffic:tick 可见 2s / 隐藏 10s 或暂停；speedtest:progress 250ms 合并 ≤4 条/秒；core:state 变化才发。
4. 窗口隐藏：前端 cancelAnimationFrame + body.is-hidden 关动画/过渡 + 通知后端降频；禁止销毁 WebView。
5. YAML 不常驻：runtime/config.yaml 用 BufWriter 流式写；serde_yaml::Value 只在解析函数内存活；运行态只留 node_key -> Option<delay>。
6. 日志：tracing_appender::non_blocking + lossy(true) + buffered_lines_limit(4096) + 单文件 2MB 守卫 + 保留 3 天。
7. 图标资源：只留 16/24/32/48/64/128/256 + icon.ico，总量 ≤200KB；托盘单独 16/32px；前端 SVG 内联；不打包中文字体。
8. NSIS：compression="lzma"、installMode="currentUser"、languages=["SimpChinese"]；不捆内核、不捆 WebView2 离线包。
9. 依赖白名单复核（10.4.11）：禁用 clap、sqlx/rusqlite、anyhow+thiserror 并存、大型 tauri-plugin；reqwest 关默认特性只留 rustls-tls+json；tokio 不用 full。

【执行流程（严格按此顺序，每步后测量）】
第 1 轮：先按 10.3.1 落地 scripts/perf-probe.ps1，跑 -Mode all 拿到「调优前」基线。
第 2 轮：构建期手段（profile + 依赖裁剪 + 图标），重测 M7/M5。
第 3 轮：运行时手段（tokio、降采样、隐藏暂停、流式写、日志），重测 M1/M2/M3/M4/M9。
第 4 轮：若仍不达标，按 10.7 的 L0→L1→L2→L3 逐级处置，每级后重测并记录。
第 5 轮：按 10.6 模板写 docs/perf-baseline.md。

【执行规则】
1. 先输出四件套（改动清单 / 预期收益与依据 / 风险点 / 每轮验证方式），确认后落盘。
2. 禁止通过改 C.11 数字来「达标」；如确需放宽，输出 CONTRACT-CHANGE-REQUEST: 并附 docs/perf-exception.md 的五项内容。
3. 每个优化都要能说出「省了多少 MB / 多少 %」的实测数字，没有数字不许写进交付说明。
4. 禁止用「应该」「理论上」描述性能结论（01 章 §8）。

【完成判定（必须全部贴出数字）】
pwsh -File .\scripts\perf-probe.ps1 -Mode all -Minutes 5 -OutJson .\docs\perf\final.json
# 退出码 0；十项全部 PASS
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test

【交付答复必须包含】
调优前后对照表（十项 × 两列）、每一级降级（如用到）的说明、docs/perf-baseline.md 内容、剩余风险。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | 用 debug 数字判断 | 结论全错 | 必须 release |
| 2 | profile 写在根 Cargo.toml | 不生效 | 写在 `src-tauri/Cargo.toml` |
| 3 | `panic="abort"` 提早开启 | 任何 panic 直接闪退，违反「失败不崩」 | P6 后段、三条件满足才开 |
| 4 | 只降采样不做流式写 | YAML 峰值仍 30MB+ | 两者都要 |
| 5 | 为省内存销毁 WebView | 重开白屏 1s+ | 隐藏即可 |
| 6 | 关日志省 CPU | 出问题无法排查 | 用 non-blocking + lossy，不要关 |
| 7 | 超预算就改契约数字 | 违反铁律 | 走 L0–L4 + CONTRACT-CHANGE-REQUEST |
| 8 | 只看主进程 RSS | 漏掉 WebView2 子树 | 两者分开报（10.3.4） |
| 9 | 测量时不预热 | 数字偏低/偏高 | 启动后 30s 再采 |

---

## 13.8 P7 · 打包发布

### 目标
产出符合 C.1 命名规则的 NSIS 安装包，完成图标、版本信息、合规声明、卸载策略与 CI。

### 前置依赖
P6 完成（体积指标已达标）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `src-tauri/tauri.conf.json` | 修改 | bundle 配置（targets nsis、icon、nsis lzma/currentUser/languages、resources 空） |
| `src-tauri/Cargo.toml` | 修改 | version 与 tauri.conf.json 一致 |
| `package.json` | 修改 | version 三处一致 |
| `src-tauri/icons/` | 修改 | 最终图标集 |
| `src-tauri/src/ui/about`（前端） | 新建 | About 页：版本、内核版本、GPL-3.0、源码链接、合规提示 |
| `src-tauri/nsis/hooks.nsh` | 新建 | 卸载询问是否保留 `%APPDATA%\ClashZhengl`（C.12.7） |
| `scripts/build.ps1` | 修改 | 体积红线守卫（10.9） |
| `.github/workflows/ci.yml` | 新建 | windows-latest 构建 + 体积守卫 |
| `docs/release-checklist.md` | 新建 | 引用 12.8 |

### 交付物
- `ClashZhengl_0.1.0_x64-setup.exe` ≤ 12 MB
- 安装 → 首启 → 卸载流程完整
- About 页含合规声明
- CI 在 PR 上跑通

### 验收标准
- [ ] 安装包名符合 `ClashZhengl_<version>_x64-setup.exe`（C.1）
- [ ] 体积 ≤ 12 MB（M7）
- [ ] 安装后磁盘 ≤ 120 MB（M8）
- [ ] 三处版本号一致
- [ ] 安装/卸载/覆盖升级均正常
- [ ] 卸载询问是否保留 `%APPDATA%\ClashZhengl`，默认保留（C.12.7）
- [ ] 无 WebView2 的机器上有引导（bootstrapper 或明确提示）
- [ ] About 页含：产品版本、mihomo 版本、GPL-3.0 声明、源码获取链接、合规提示
- [ ] 首启合规提示（不提供任何节点/订阅）
- [ ] 默认非管理员运行（`installMode: currentUser`）
- [ ] 未捆绑内核（策略 a）或已按 C.12.5(b) 附 LICENSE
- [ ] CI 通过；体积守卫生效

### 预估工时
1.5 天。

### 执行指令块

```text
你是资深 Windows 打包/发布工程师，负责 Clash Zhengl 的 P7 阶段：NSIS 打包与发布准备。

【必读上下文】
- C.1 是产品身份唯一依据：productName "Clash Zhengl"、可执行 clash-zhengl.exe、identifier com.clashzhengl.app、
  安装包名 ClashZhengl_<version>_x64-setup.exe、SemVer 首版 0.1.0。
- C.12 是安全与合规基线：默认非管理员；内核 GPL-3.0 以独立 sidecar 调用、不修改不静态链接；
  分发策略二选一（推荐 (a) 不捆绑、首启引导下载）；卸载询问是否保留 %APPDATA%\ClashZhengl，默认保留。
- C.11 的 M7（安装包 ≤12MB）、M8（常驻磁盘 ≤120MB）是硬指标。
- 10 章 10.4.8 是 NSIS 压缩与瘦身施工依据；10.4.7 是图标资源裁剪；10.9 是构建期体积守卫与 CI 片段。
- 11 章是打包发布细节依据（图标与版本信息、WebView2 引导、签名取舍、内核分发、About、CI、更新检查）。

【技术约束】
- Tauri 2 bundle：targets ["nsis"]、compression "lzma"、installMode "currentUser"、languages ["SimpChinese"]、
  displayLanguageSelector false、resources []（不要把 docs/examples 打进去）。
- 不捆绑内核（策略 a）；不捆绑 WebView2 离线安装器（约 130MB）；
  若用户明确要求捆绑内核，必须同时附 mihomo 的 LICENSE、版本与源码获取链接（C.12.5b）。
- 不引入遥测/统计/崩溃上报 SDK；不自动下载更新（更新检查只提示，由用户点击）。
- WebView2 缺失时给出明确中文引导，不许静默失败。
- 版本号三处一致：src-tauri/tauri.conf.json、src-tauri/Cargo.toml、package.json。

【本阶段要交付的文件】
1. src-tauri/tauri.conf.json（bundle 最终配置）
2. src-tauri/nsis/hooks.nsh（卸载时询问是否保留 %APPDATA%\ClashZhengl，默认保留）
3. About 页前端（版本 / 内核版本 / GPL-3.0 / 源码链接 / 合规提示）
4. scripts/build.ps1（构建 + 体积红线：exe >15MB 或 setup >12MB 直接失败）
5. .github/workflows/ci.yml（windows-latest：fmt/clippy/test/tsc/build + 体积守卫）
6. docs/release-checklist.md（引用 12 章 12.8 的发布前检查表）

【执行规则】
1. 先输出四件套（改动清单 / 关键配置片段 / 风险点 / 验证方式），确认后落盘。
2. 给出完整的 tauri.conf.json bundle 片段与 hooks.nsh 全文，不要省略。
3. 说明代码签名的取舍（未签名 → SmartScreen 警告，如何在文档中告知用户；不做假签名）。
4. 契约未覆盖的输出 CONTRACT-CHANGE-REQUEST:。

【完成判定（自己跑并贴结果）】
cargo tauri build
$s = Get-ChildItem src-tauri\target\release\bundle\nsis -Filter '*_x64-setup.exe'
$s | Select-Object Name, @{n='MB';e={[math]::Round($_.Length/1MB,2)}}
pwsh -File .\scripts\perf-probe.ps1 -Mode disk
# 手工：干净 Win11 虚拟机安装 → 首启 → 加订阅 → 开代理 → 卸载 → 检查残留

【交付答复必须包含】
产物路径与体积、安装/卸载实测结论、About 页截图或文案全文、未签名的影响说明、剩余风险。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | 三处版本号不一致 | 安装包名与 About 显示不同版本 | 构建脚本统一读取 |
| 2 | NSIS 用默认压缩 | 包体多 3~5 MB | `lzma` |
| 3 | `installMode: "perMachine"` | 每次安装要 UAC，违反非管理员基线 | `currentUser` |
| 4 | 把 `docs/` 打进 resources | 包体莫名变大 | `resources: []` |
| 5 | 卸载直接删 `%APPDATA%` | 用户丢失全部订阅 | 询问，默认保留 |
| 6 | 捆绑内核不附 LICENSE | GPL 违规 | 策略 a 或附 LICENSE + 源码链接 |
| 7 | WebView2 缺失无提示 | 双击没反应 | 明确中文引导 |
| 8 | 忘了图标 | 安装器显示默认 Tauri 图标 | 检查 `icon` 数组 |
| 9 | CI 里跑性能门禁 | Runner 噪声大，数字无意义 | CI 只守包体 |
| 10 | 声称已签名但实际没有 | 用户看到 SmartScreen 警告 | 如实说明 |

---

## 13.9 P8 · 验收收尾

### 目标
跑完全部回归与异常清单，产出验收报告与 Release Notes，交付可发布版本。

### 前置依赖
P7 完成。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `docs/acceptance-report.md` | 新建 | 12.7 全阶段勾选表 + 实测凭证 |
| `docs/release-notes.md` | 新建 | 功能、已知问题、未验证项 |
| `docs/perf-baseline.md` | 修改 | 最终数字 |
| `tests/` 下测试 | 修改 | 补齐遗漏用例至 90+ |
| `src-tauri/src/**` | 修改 | 仅修缺陷，不加功能 |

### 交付物
- 验收报告（含 20 条异常、性能十项、各阶段勾选）
- Release Notes（含已知问题与「未验证」项）
- 24 小时挂机报告

### 验收标准
- [ ] 12.6 的 20 条异常全部通过（逐条有结论）
- [ ] 12.2 的 L1–L7 全绿（L1 ≥ 90 个单测）
- [ ] 12.8 发布前检查表全勾（代码/安全/功能/文档四节）
- [ ] C.11 十项指标全部达标（`perf-probe -Mode all` 退出码 0）
- [ ] 24 小时挂机：自动更新 + 自动测速 + 自动切换持续工作，RSS 前后差 ≤ 10 MB
- [ ] 脱敏复查通过（`logs/` 与诊断包全文搜无明文）
- [ ] 12.4 契约测试通过（无契约外命令/错误码）
- [ ] 缺陷分级表已过一遍，无未处置 P0/P1
- [ ] Release Notes 明确列出「未验证」项
- [ ] 新装机器完整流程一遍通过

### 预估工时
2 天（含 24h 挂机的等待时间可并行做文档）。

### 执行指令块

```text
你是 Clash Zhengl 的交付负责人，负责 P8 验收收尾。本阶段只修缺陷，不加功能。

【必读上下文】
- 12-testing-and-acceptance.md 是本章依据：12.2 测试矩阵、12.3 Mock 服务器与期望行为表、12.4 契约一致性测试、
  12.6 必测异常清单 20 条、12.7 分阶段验收勾选表、12.8 发布前检查表、12.9 缺陷分级。
- 10 章的 C.11 十项指标与 docs/perf-baseline.md 模板。
- C.8 错误码、C.9 日志脱敏、C.12 安全合规基线。

【执行流程（按顺序，每步留证）】
第 1 步 代码门禁：cargo fmt --check / cargo clippy --all-targets -- -D warnings / cargo test /
       pnpm tsc --noEmit / pnpm biome check / cargo tauri build。全绿才继续。
第 2 步 契约一致性：跑 cargo test --test contract_commands 与前端 contract.test.ts；
       全仓扫描 E_ 前缀字面量必须在 C.8 白名单内；扫描 invoke('...') 必须在 C.6 白名单内。
       发现漂移 → 按 P0 处理（会让后续所有 Agent 会话一起漂移）。
第 3 步 性能：pwsh -File .\scripts\perf-probe.ps1 -Mode all -Minutes 5 -OutJson .\docs\perf\final.json
       退出码必须 0；数字写入 docs/perf-baseline.md（按 10.6 模板，含内核内存参考值）。
第 4 步 异常清单 20 条：逐条按 12.6 表执行，记录「操作 / 实际现象 / 实际错误码 / 是否可恢复」。
       任何一条不符 → 登记缺陷（按 12.9 分级），P0/P1 必须修完再继续。
第 5 步 手工全流程：干净 Win11 虚拟机 → 安装 → 首启合规提示 → 加 3 条订阅 → 10s 内显示流量与到期 →
       开系统代理 → 浏览器可访问外网 → 设 15min 订阅周期 + 5min 测速周期 → 观察 2 个周期 →
       自动切换到最低延迟 → 关窗口进托盘 → 托盘菜单全试一遍 → 退出 → 无残留进程 → 卸载。
第 6 步 脱敏复查：抓 logs/ 与 diagnostics/ 的 zip，全文搜 token= / secret / 32 位 hex / https?:// 带查询参数，
       必须无明文命中；有命中 → P0。
第 7 步 24 小时挂机：记录起止 RSS、CPU 平均、自动更新次数、测速次数、切换次数；RSS 前后差 ≤10MB。
第 8 步 文档：写 docs/acceptance-report.md 与 docs/release-notes.md。

【硬性要求】
1. 每一条验收项必须有「凭证」：截图路径、日志片段、命令输出或数字。没有凭证不算通过。
2. 未验证的项必须写「未验证」，禁止写「应该没问题」「理论上通过」（01 章 §8）。
3. 任何缺陷按 12.9 分级；P0 定义包含：崩溃 / 数据丢失 / 凭据泄露 / 无法启动 / 系统代理无法恢复 / 契约偏离。
4. 不得为了「全部打勾」而修改 C.11 数字或删掉验收项；做不到就如实写未达标 + 处置方案。
5. 契约需要变更时输出 CONTRACT-CHANGE-REQUEST:，不要改 C 章。

【完成判定（全部贴出证据）】
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
pnpm tsc --noEmit && pnpm biome check
cargo tauri build
pwsh -File .\scripts\perf-probe.ps1 -Mode all -Minutes 5 -OutJson .\docs\perf\final.json
# 附：20 条异常清单结果表、24h 挂机记录表、脱敏复查输出

【交付答复必须包含】
验收报告路径、十项性能数字、20 条异常结论、缺陷清单与处置、未验证项清单、发布建议（可发/需修后发）。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | 只在开发机测 | 换机器各种路径/权限问题 | 干净虚拟机 + 中文用户名 + 非管理员 |
| 2 | 打勾无凭证 | 无法复盘 | 每条留数字/日志/截图 |
| 3 | 把「应该通过」写进报告 | 违反铁律 | 写「未验证」 |
| 4 | 契约漂移没人管 | 后续 Agent 会话全跟着错 | 契约测试作为 P0 门禁 |
| 5 | 24h 挂机前不重启 | 内存基线不准 | 冷启动开始计时 |
| 6 | 脱敏只搜 `token=` | 漏掉 32 位 hex secret | 多模式搜（12.11 的命令） |
| 7 | 遗留 P1 缺陷就发布 | 用户踩坑 | P0/P1 必修 |
| 8 | Release Notes 只写功能 | 用户遇到已知问题无预期 | 必须写已知问题与限制 |

---

## 13.10 卡片之间的一致性自查（每张卡交付前都过一遍）

- [ ] 本卡新增/使用的 IPC 命令全部在 C.6 表中，**逐字一致**（含下划线位置）
- [ ] 本卡新增/使用的事件名全部在 C.7 表中，格式为 `<domain>:<event>`
- [ ] 本卡返回的错误码全部在 C.8 表中；需要新增已提 `CONTRACT-CHANGE-REQUEST:`
- [ ] 本卡涉及的文件路径全部能在 C.2 目录结构里找到；需要新目录已说明理由
- [ ] 本卡涉及的时间/单位/精度符合 C.10（Unix 秒、字节、1024 进制、延迟 u32 ms）
- [ ] 本卡未违反 01 章 §8 的禁止项（不改 mihomo 源码、不 0.0.0.0、不空 secret、不集成遥测、不 log 凭据）
- [ ] 本卡的验收项已被 12.7 对应小节覆盖（无遗漏、无重复冲突）
- [ ] 本卡交付说明含「完成项 / 未完成项 / 已知问题 / 下一步 / 需确认问题」五项

---

## 13.11 阶段依赖与并行建议

```text
P0 ──► P1 ──► P2 ──► P3 ──► P4 ──► P5 ──► P6 ──► P7 ──► P8 ──► P9(必做)
       │                    │
       └── P2 的 userinfo.rs / parse.rs 纯函数可并行（无 IO 依赖）
                            └── P5 的静态页面骨架可在 P4 期间先行（不接后端）
```

| 并行机会 | 条件 | 风险 |
| --- | --- | --- |
| P1 期间写 P2 的 `userinfo.rs` | 纯函数，无依赖 | 无 |
| P4 期间搭 P5 的静态页面 | 只做 DOM/CSS，先 mock 数据 | 接口对齐要等 P4 定稿，可能返工 |
| P6 的 `perf-probe.ps1` 在 P0 就落地 | 脚本独立 | 无（推荐提前做） |
| P7 的 CI 骨架在 P2 就可以搭 | CI 独立 | 无（推荐提前做） |
| P9 的 `check-contract.ps1` 在 P2 就可以先写 | 只依赖 C 章与代码文本 | 无（推荐提前，越早拦住漂移越省事） |

> **P9 的并行约束**：P9 可与 P7（打包）/P8（验收）并行开工，但**必须在验收收尾结束前完成**——
> 它是「契约对账 + 性能台账 + 变更流程」三项基建，晚于验收就等于本版已经带着漂移风险交付了。

**不建议并行**：P2 的 manager（合并）与 P1 的端口/secret（强依赖）；P4 的 selector 与 P3 的 runner（事件顺序会乱）。

---

## 13.12 一句话执行模板

> 「读 01 与 C，执行 `13-iteration-task-cards.md` 的 **P{N}** 那张卡。
> 先给四件套等我确认，再落盘。完成后按 12 章对应小节自检并贴出凭证。
> 契约冲突输出 CONTRACT-CHANGE-REQUEST:，不要自创。」

---

## 13.13 P9 · 交付后运维（必做）

> **何时做**：P8 验收通过、首个版本发布之后。若项目只做一次性交付可跳过；
> 但只要还会继续迭代（加功能、追内核版本、修 bug），这张卡**必须做**——否则下一轮 AI 会话会从头开始漂。

### 目标
把「契约不漂移」和「性能有账本」两件事固化成脚本与流程，让后续每一轮迭代都能自动被拦住。

### 前置依赖
P8 完成（已有可运行的验收体系与 `docs/acceptance-report.md`）。

### 改动文件清单

| 文件 | 动作 | 要点 |
| --- | --- | --- |
| `docs/perf-log.md` | 新建 | 每阶段/每版本的性能账本（C.11 v1.1 的 100%–120% 过渡记录也进这里） |
| `scripts/check-contract.ps1` | 新建 | 从 `appendices/C-conventions.md` 抽出命令/事件/错误码三张表，与实际代码对账 |
| `.github/workflows/ci.yml` | 修改 | 加 `check-contract.ps1` 步骤（PR 必跑） |
| `docs/contract-change.md` | 新建 | 契约变更流程：谁能改、怎么提、变更日志表怎么维护 |
| `README.md` | 修改 | 指向上述两个文档 |

### 交付物
- `pwsh -File .\scripts\check-contract.ps1` 本地可跑，发现漂移时**退出码非 0**
- CI 在 PR 上执行该脚本
- `docs/perf-log.md` 有至少一条真实记录
- `docs/contract-change.md` 写清「改 C 章的唯一入口」

### 验收标准
- [ ] `check-contract.ps1` 能从 C 章文本中解析出 30 个命令 / 10 个事件 / 17 个错误码
- [ ] 代码侧多出一个契约外命令 → 脚本报错并非 0 退出
- [ ] 代码侧少一个契约内命令 → 脚本报错并非 0 退出（防止「只删代码不改契约」）
- [ ] 检测到 `ui_visibility_set` → 直接失败（C.6 明确不做）
- [ ] CI 有该步骤且能在 PR 上拦住
- [ ] `docs/contract-change.md` 含变更日志表模板与 `CONTRACT-CHANGE-REQUEST:` 提交流程
- [ ] `docs/perf-log.md` 含一键追加命令（不必手写表格）

### 预估工时
0.5 天。

### 执行指令块

```text
你是 Clash Zhengl 的工程负责人，负责 P9 交付后运维：把「契约不漂移」和「性能有账本」固化成脚本与流程。
本卡**必做**，可与 P7/P8 并行，但必须在验收收尾前完成。

【必读上下文】
- C-conventions.md（现 v1.2）是唯一权威。它的 C.6 命令表、C.7 事件表、C.8 错误码表是三个白名单。
- 12-testing-and-acceptance.md 的 12.4 已有 Rust/TS 两处契约测试——但那是「代码内白名单」，
  本卡要做的是「从 C 章原文解析后对账」，两者互补：C 章改了而代码没跟上时，前者查不出来。
- 13 章各卡末尾的 CONTRACT-CHANGE-REQUEST: 约定。

【要交付的四个文件】
1. scripts/check-contract.ps1（pwsh 7，零外部依赖）：
   - 用正则从 appendices/C-conventions.md 抽三张表：
     C.6 表格首列的 `xxx_yyy` 命令名；C.7 表格首列的 `domain:event` 事件名；C.8 表格首列的 `E_XXX` 错误码。
   - 从 src-tauri/src 递归扫 .rs，抽出 generate_handler! 里注册的命令名、emit(...) 里的事件名、字面量 E_ 错误码。
   - 从 src/ 扫 .ts，抽出 invoke('...') 的命令名。
   - 三方对账：C 章有而代码无 → 失败；代码有而 C 章无 → 失败；命中 ui_visibility_set → 直接失败。
   - 输出清晰 diff（缺哪个、多哪个、在哪个文件哪一行），退出码 0/1。
   - 提供一个 -Fix 参数：只打印建议，不自动改文件（禁止脚本改源码）。
2. .github/workflows/ci.yml：在 windows-latest 上、fmt/clippy/test/tsc 之后加一步跑 check-contract.ps1。
3. docs/contract-change.md：写清 ①只有用户/Lead 能改 C 章；②编码 Agent 必须提 CONTRACT-CHANGE-REQUEST: 的格式；
   ③变更日志表的维护规则（版本号、日期、一句话说清改了什么、影响哪些章节）；
   ④改契约后必须同步的三个地方（12.4 白名单、13 章对应卡、附录 A/B 的对齐记录表）。
4. docs/perf-log.md：表格列 = 日期/版本/阶段/指标/预算/实测/百分比/结论/改进项；
   并给一条 pwsh 追加命令样例（从 perf-probe 的 JSON 读出并 append）。

【执行规则】
1. 先输出四件套，确认后落盘。
2. check-contract.ps1 必须能在**不做任何构建**的前提下跑（纯文本解析，不 invoke cargo）。
3. 要容忍 C 章表格的书写差异（单元格里带反引号、带括号说明），不要把解析写得过脆。
4. 脚本只读，禁止修改任何源文件（-Fix 也只打印建议）。
5. 契约若确实需要变更，输出 CONTRACT-CHANGE-REQUEST:，不要自己改 C 章。

【完成判定（自己跑并贴结果）】
pwsh -File .\scripts\check-contract.ps1            # 期望退出码 0
# 故意注入一个假命令名到某个 .rs 文件，重跑，期望退出码 1 且报错信息清晰；验证后撤销该注入
# 贴出：解析到的命令数/事件数/错误码数是否等于 30 / 10 / 17

【交付答复必须包含】
完成项/未完成项/已知问题/下一步建议/需确认问题；贴 check-contract.ps1 的正常输出与一次故意失败输出。
```

### 常见坑

| # | 坑 | 现象 | 规避 |
| --- | --- | --- | --- |
| 1 | 从代码里抽白名单而不是从 C 章抽 | C 章改了代码没改时**查不出来**（自证清白） | 必须解析 C 章原文做基准 |
| 2 | 正则写太严 | C 章格式微调就全抓不到，脚本变哑巴 | 容忍反引号与括号说明；先断言三条数量 30/10/17 不匹配就直接报错 |
| 3 | 脚本顺手改源码 | 违反「契约变更走流程」 | 只读，`-Fix` 仅打印建议 |
| 4 | 只在本地跑不进 CI | 靠人自觉 = 迟早漂 | PR 必跑 |
| 5 | `perf-log.md` 手写 | 三版之后就没人填了 | 给出一条可复制的 append 命令 |
| 6 | 契约变更不留日志 | 半年后没人知道某个字段为何存在 | `contract-change.md` 强制要求维护变更日志表 |

---

## 契约对齐记录（v1.4）

| 契约条目 | 本章落点 | 状态 |
| --- | --- | --- |
| v1.1 · `core_versions_list` / `switch_history` | P1 卡（命令清单 + 交付物）、P4 卡（`switch_history`） | ✅ 已落地 |
| v1.1 · `core:install-progress` / `sysproxy:state` | P1 卡技术约束 + 验收标准 + 常见坑 #11–14 | ✅ 已落地 |
| v1.1 · `RuntimeState.sysproxy`（含 `managed_by_us`） | P1 卡必读上下文 + 验收标准 | ✅ 已落地 |
| v1.1 · `TrafficSummary` 聚合 | P5 卡（`traffic_summary` 两级渲染、`has_missing`、`stale`） | ✅ 已落地 |
| v1.1 · `compliance_accepted_at` / schema v2 | P5 卡验收（首启合规提示），P0 卡（schema_version 起始值） | ✅ 已落地 |
| v1.1 · `runtime/download/` 续传 | P1 卡技术约束（`.part` + `.part.json`） | ✅ 已落地 |
| v1.2 · `SwitchRecord.to_delay: Option<u32>` | P4 卡验收 + 指令块 + 常见坑 #11；P5 卡历史列表 `—` | ✅ 已落地 |
| v1.2 · `manual_override_min` + 手动覆盖 | P4 卡改 4 处（清单/验收/指令块/坑 #12–14）+ P5 卡 UI 明示 | ✅ 已落地 |
| v1.2 · 手动锁定后自动切换跳过该组 | P4 验收标准中的独立可勾选项 | ✅ 已落地 |
| v1.2 · scope 组必须 `type: select` | P4 验收 + 指令块 + 常见坑 #16 | ✅ 已落地 |
| **v1.3 · `switch_apply_best` 返回 `Option<{primary, all}>`，`primary == all[0]`** | P4 卡指令块（实现要点整段重写）+ 验收 2 条 | ✅ 已落地 |
| **v1.3 · 无生效返回 `None`，不存在 `primary: null`** | P4 卡验收独立可勾选项 + 指令块明确「不得返回 `{primary: null, all: []}`」 | ✅ 已落地 |
| **v1.3 · `primary` = 首个实际生效组**（按生效顺序，非 group_names 序、非延迟序） | P4 卡验收 + 指令块「不要自创规则」 | ✅ 已落地 |
| **v1.3 · §C.6.1 冻结面五类型不是 IPC** | P4 卡改动清单（`kernel/api.rs` 行）+ 指令块执行规则第 1 条 + 验收独立勾选项 | ✅ 已落地 |
| **v1.3 · `kind` 取 API 值 `Selector`（非 YAML `select`），比较须大小写敏感** | P4 卡**常见坑 #1（头号坑，含静默失效说明）** + 技术约束独立条目 + 交付物 + 验收 2 条 | ✅ 已落地 |
| **v1.3 · P9 改为必做** | 13.0 总览表 + 关键路径 + 13.11 并行约束 + 13.13 标题与指令块 | ✅ 已落地 |
| v1.3 · `ApiError` 五变体 | 由 08 章实现（本章 P4 卡引用 C.6.1 冻结面） | ✅ 已引用 |

| **v1.4 · 配置取值钉死**：`find-process-mode: strict`、`ipv6: true`、`dns.enable: false` | P2 卡验收独立可勾选项 + 技术约束「配置取值硬约束」段 + 常见坑 #13 / #15 | ✅ 已落地 |
| **v1.4 · §A.6.4 禁止生成自动策略组** | P2 卡验收独立可勾选项 + 技术约束段（`assert_no_auto_groups` fail-fast）+ 常见坑 #14 | ✅ 已落地 |
| **v1.4 · `SelectorApi` 的 `ApiFuture` dyn 兼容形式** | P4 卡改动清单 + 指令块第 4 项（给出完整 trait 定义、禁止 `async fn`、禁止 `async-trait` 宏） | ✅ 已落地 |

### 未决项

**无。** 原 4 条未决项已由 C 章 v1.3 全部裁决，v1.4 另补配置取值与 `ApiFuture` 形式：
1. `SelectorApi` / `GroupInfo` / `SelectableGroup` / `SwitchRuntimeLocks` / `ApiError` 已纳入 §C.6.1 冻结面，
   本章按「内部接口、不是 IPC 命令、不得新增 invoke」表述（另注：`SelectableGroup` 字段为 `members`，非 `candidates`）；
2. `primary` 选取规则定稿为「`primary == all[0]` = 首个实际生效的组」；
3. 无生效时整体返回 `None`，契约明确不存在 `primary: null` 形态；
4. P9 已改为必做卡，纳入关键路径。
