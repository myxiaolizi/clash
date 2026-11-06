# 08 · 内核管理与系统代理（Kernel Supervisor & System Proxy）

> **适用阶段**：P1（内核可启动 + 系统代理可开关是 MVP 的物理前提）
> **前置阅读**：`01-master-prompt.md`、`appendices/C-conventions.md`（下称 C 章）
> **本章职责**：定义 mihomo 内核的**获取、启动、监护、退出**全生命周期，以及 **WinINET 系统代理**的
> 开关、刷新、崩溃恢复守卫。本章所有命名、路径、IPC、错误码严格对齐 C 章，**不得自行发明**。
> **本章不负责**：订阅解析与合并（03/04 章）、测速与切换策略（06/07 章）、打包（11 章）。

---

## 08.0 文件映射（对齐 C.2 目录结构）

| 关注点 | 文件 | 说明 |
| --- | --- | --- |
| 进程生命周期、健康检查、崩溃重启 | `src-tauri/src/kernel/supervisor.rs` | 状态机唯一实现处 |
| external-controller REST 客户端 | `src-tauri/src/kernel/api.rs` | Bearer secret、超时、错误归一 |
| 端口探测与分配 | `src-tauri/src/kernel/ports.rs` | 纯逻辑为主，可单测 |
| 内核下载/校验/解压/版本切换/清理 | `src-tauri/src/kernel/fetch_core.rs` | 有网络 IO，需可取消 |
| WinINET 系统代理 | `src-tauri/src/system/sysproxy.rs` | 注册表 + `InternetSetOptionW` |
| 开机自启 | `src-tauri/src/system/autostart.rs` | HKCU Run |
| 单实例 | `src-tauri/src/system/single_instance.rs` | 命名互斥 + 唤起窗口 |
| 托盘（构建与事件） | `src-tauri/src/system/tray.rs` | 交互细节见 09 章 |
| IPC 薄壳 | `src-tauri/src/commands/core.rs` | `core_*` / `sysproxy_set` |

**分层铁律**：`commands/core.rs` 内**禁止**出现 `Command::new`、注册表读写、`LazyLock` 端口探测；
只做参数校验 + 调 `kernel::supervisor` / `system::sysproxy` + DTO 转换。

### 08.0.1 本章引入的依赖（01 §5.5 / C.13 要求逐项说明）

| crate | 用途 | 体积影响（release，约） | 许可证 | 可否避免 |
| --- | --- | --- | --- | --- |
| `windows`（feature 裁剪） | WinINET / Job Object / 互斥体 / 注册表 | +120 KB（只开 `Win32_Networking_WinInet`、`Win32_System_Threading`、`Win32_System_Console`、`Win32_Security`） | MIT / Apache-2.0 | 不可（无标准库等价物） |
| `zip`（仅 `deflate`） | 解压内核 zip | +90 KB | MIT | 不可（或改用 `flate2` 手写 zip 中央目录，不值得） |
| `sha2` | SHA256 校验 | +30 KB | MIT / Apache-2.0 | 不可 |
| `hex` | hex 编解码 | +5 KB | MIT / Apache-2.0 | 可（20 行手写），但为 5 KB 不值 |
| `getrandom` | 生成 secret | 已间接依赖（rand 生态） | MIT / Apache-2.0 | 可（`BCryptGenRandom`），但 getrandom 在 Windows 就是调它 |
| `fastrand` | 端口随机 | +3 KB | MIT / Apache-2.0 | 可（`getrandom` 直接取 u16），**建议合并，去掉 fastrand** |
| `semver` | 版本目录排序 | +8 KB | MIT / Apache-2.0 | 可（内核版本格式固定，手写三段比较）；**建议去掉** |

> **净增预算**：约 +250 KB（未压缩），远低于 12 MB 包体预算的 2%。上表末两条为可合并项，
> 实施时优先去掉 `fastrand` 与 `semver`，把新增依赖压到 5 个。
> 所有新依赖必须登记进 `THIRD-PARTY-NOTICES.md`（11 章）。

### 08.0.2 关于 `ErrCode` 与 C.8 的关系

本章代码出现 `ErrCode::CoreStartFailed` 之类的**内部枚举**，其唯一作用是编译期防止拼错字符串。
序列化到前端时**必须**输出 C.8 的字符串 code：

```rust
// domain/errors.rs
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub enum ErrCode {
    #[serde(rename = "E_CORE_NOT_FOUND")]  CoreNotFound,
    #[serde(rename = "E_CORE_START_FAILED")] CoreStartFailed,
    #[serde(rename = "E_CORE_API_UNAUTH")] CoreApiUnauth,
    #[serde(rename = "E_CORE_PORT_EXHAUSTED")] CorePortExhausted,
    #[serde(rename = "E_STORE_IO")]        StoreIo,
    #[serde(rename = "E_PERM_ADMIN")]      PermAdmin,
    #[serde(rename = "E_UNSUPPORTED")]     Unsupported,
    // …其余见 C.8，一个不多一个不少
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrCode,          // 序列化为字符串，前端只看到 C.8 的 code
    pub message: String,        // 中文，C.8 文案
    pub detail: Option<String>, // 仅供日志，禁止含凭据
    pub retryable: bool,
}
```

> **禁止**在 `ErrCode` 里增加 C.8 之外的变体；需要新语义时走 `CONTRACT-CHANGE-REQUEST:`。

---

## 08.1 内核资产与版本策略

### 08.1.1 资产命名规则

mihomo 官方 Release 的 Windows amd64 资产形如：

```
https://github.com/MetaCubeX/mihomo/releases/download/v1.19.33/mihomo-windows-amd64-compatible-v1.19.33.zip
```

| 资产名（以 v1.19.33 为例） | 兼容性取向 | 选用优先级 | 说明 |
| --- | --- | --- | --- |
| `mihomo-windows-amd64-compatible-v1.19.33.zip` | 最宽（老 CPU / 老系统基线） | **1（默认）** | 体积略大、性能差异在代理场景可忽略；**默认选它最稳** |
| `mihomo-windows-amd64-v1.19.33.zip` | 标准 amd64 | 2（回退） | 首选 404 时回退 |
| `mihomo-windows-amd64-v1-v1.19.33.zip` | GOAMD64=v1 微架构 | 3（回退） | 注意 `-v1-` 与版本号 `v1.19.33` 的**双 v 歧义**，解析必须用锚定正则 |
| `mihomo-windows-amd64-v2-…zip` / `-v3-…zip` | 需 SSE4.2 / AVX2 等 | **不选** | 老机器会直接 `illegal instruction`，禁止默认下发 |

**资产名解析（唯一实现，禁止在别处拼字符串）**

```rust
// kernel/fetch_core.rs
const ASSET_CANDIDATES: [&str; 3] = [
    "mihomo-windows-amd64-compatible-{ver}.zip",
    "mihomo-windows-amd64-{ver}.zip",
    "mihomo-windows-amd64-v1-{ver}.zip",
];

fn asset_url(version: &str, idx: usize) -> String {
    format!(
        "https://github.com/MetaCubeX/mihomo/releases/download/{ver}/{name}",
        ver = version,
        name = ASSET_CANDIDATES[idx].replace("{ver}", version)
    )
}
```

> **不硬编码 x64**：C.1 要求「代码中不得硬编码 x64 假设」。实现时由
> `std::env::consts::ARCH` 映射出 `amd64` / `arm64`（本产品只发布 x64，但函数签名保留 arch 参数，
> 且 `arm64` 分支返回 `E_UNSUPPORTED`），便于将来扩展时只改一处。

### 08.1.2 版本目录布局

```
%LOCALAPPDATA%\ClashZhengl\                 # C.3：Local，不漫游
├─ runtime\
│  ├─ config.yaml                           # 04 章产出，每次启动/更新重建
│  ├─ secret                                # 32 hex，每次启动重新生成
│  ├─ sysproxy.journal.json                 # 系统代理恢复守卫日志（见 08.10）
│  ├─ switch-history.json                   # 07 章切换历史（最近 200 条滚动丢弃）
│  ├─ download\                             # 下载与解压临时区（可随时整目录删除）
│  │  ├─ mihomo-v1.19.33.zip.part
│  │  ├─ mihomo-v1.19.33.zip.part.json      # 断点续传元数据
│  │  └─ extract-v1.19.33\                  # 解压暂存，完成后整体 rename
│  └─ core\
│     ├─ v1.19.32\mihomo.exe
│     └─ v1.19.33\mihomo.exe                # 当前使用
```

> **`runtime\switch-history.json` 的归属**（C.3 v1.1 新增）：由 **07 章**的
> `speedtest/selector.rs` 写入与裁剪（`Vec<SwitchRecord>`，仅保留最近 200 条），
> `switch_history` 命令（C.6）从该文件读。08 章只负责**不误删它**：
> 内核版本 GC（08.1.3）与 `download\` 清理都限定在各自子目录内，
> **不得对整个 `runtime\` 做 `remove_dir_all`**。卸载时的删除走 11 章的 NSIS 钩子。

**当前版本如何确定（不新增状态文件）**：直接复用 C.5 的 `AppSettings.core_version`。

```rust
/// None => 已安装的最新（按 SemVer 排序）；Some(v) => 指定版本，缺失则 E_CORE_NOT_FOUND
fn resolve_core_exe(settings: &AppSettings, root: &Path) -> Result<PathBuf, AppError> {
    let dir = match settings.core_version.as_deref() {
        Some(v) => root.join("core").join(v),
        None => latest_installed(&root.join("core"))?, // 读目录名 + 解析 SemVer 取最大
    };
    let exe = dir.join(if cfg!(windows) { "mihomo.exe" } else { "mihomo" });
    if !exe.is_file() {
        return Err(AppError::new(ErrCode::CoreNotFound, "未安装内核，请先下载"));
    }
    Ok(exe)
}

/// C.6 `core_versions_list`：设置页版本下拉的唯一数据源
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsList {
    pub installed: Vec<String>,       // 扫 runtime\core\ 下的版本目录，SemVer 降序
    pub current: Option<String>,      // settings.core_version，None 时取 installed[0]
    pub available: Vec<String>,       // 来自 GitHub Release；见下
}

#[tauri::command]
pub async fn core_versions_list(state: tauri::State<'_, AppState>) -> Result<VersionsList, AppError> {
    let installed = scan_installed(&state.paths.core_root());      // 本地，必有
    let current = state.settings.read().core_version.clone()
        .or_else(|| installed.first().cloned());
    // C.6 明文：available 需网络，**失败返回空数组且不报错**（不得让设置页因此报错）
    let available = match list_releases_from_github(&state.http).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(target: "kernel", "list releases failed err={}", e); // 只记日志
            Vec::new()
        }
    };
    Ok(VersionsList { installed, current, available })
}
```

> **`available` 的边界**：它是「可安装列表」，来源为 GitHub API 的 releases（只取带
> `mihomo-windows-amd64-compatible-*` 资产的 tag）。无网络/被墙/API 限额时返回空数组，
> UI 只显示本地已装版本 + 「手动输入版本号」入口，**不得弹错误框**（C.6 明文）。
> 若将来要缓存该列表，落到 `%LOCALAPPDATA%\ClashZhengl\runtime\` 下新文件，**需先提
> CONTRACT-CHANGE-REQUEST 加进 C.3** —— 本章不擅自新增路径。

### 08.1.3 版本清理策略（铁律 6「不留垃圾」）

| 时机 | 动作 | 保留策略 |
| --- | --- | --- |
| 内核启动成功后 | 清理 `download\` 整目录 | 全删（临时区） |
| 安装新版本成功后 | 清理旧版本目录 | 保留**当前 + 上一版**，其余删除 |
| 应用启动时（低频） | 清扫超过 7 天的残留 `.part` / `extract-*` | 全删 |
| 卸载时 | 由 NSIS 钩子询问后删除 `runtime\` | 见 11 章 |

```rust
fn gc_core_versions(core_root: &Path, keep_current: &str, keep_prev: Option<&str>) -> io::Result<Vec<String>> {
    let mut removed = Vec::new();
    for ent in fs::read_dir(core_root)? {
        let ent = ent?;
        let name = ent.file_name().to_string_lossy().to_string();
        if name == keep_current || Some(name.as_str()) == keep_prev { continue; }
        // 只删“看起来像版本目录”的项，避免误删用户手工放的东西
        if SemVer::parse(&name).is_none() { continue; }
        match fs::remove_dir_all(ent.path()) {
            Ok(()) => removed.push(name),
            Err(e) => tracing::warn!(target: "kernel", "gc failed dir={} err={}", name, e),
        }
    }
    Ok(removed)
}
```

> **禁止**在 `gc` 中删除 `keep_current`；删除前必须 `assert!(!is_core_running())`，否则 Windows 上
> 会因文件被占用返回 `ERROR_ACCESS_DENIED`（错误码 5），此时记 `WARN` 并跳过，不重试。

---

## 08.2 内核下载：校验、断点续传、代理、原子落位

### 08.2.1 流程（`fetch_core.rs` 的 `install_core`）

| 步 | 动作 | 失败映射 |
| --- | --- | --- |
| 1 | 解析版本：`version` 入参 > `settings.core_version` > 内置 `DEFAULT_CORE_VERSION` | — |
| 2 | 若 `core\<ver>\mihomo.exe` 已存在且可执行 → 直接返回（幂等） | — |
| 3 | 依 `ASSET_CANDIDATES` 顺序 `HEAD` 探测，取第一个 200 | `E_HTTP_STATUS` |
| 4 | 读取 `.part.json`，有则带 `Range: bytes=<n>-` 续传 | — |
| 5 | 流式下载到 `.part`，每 256 KB 发一次 `core:install-progress`（phase=`download`，合并 ≤2 条/秒） | `E_NET_TIMEOUT` / `E_NET_TLS` / `E_NET_PROXY` |
| 6 | SHA256 校验（见 08.2.2），进度发 phase=`verify` | `E_CORE_START_FAILED`（detail: 校验和不匹配） |
| 7 | 解压到 `download\extract-<ver>\`，只取 `mihomo.exe`（拒绝路径穿越），进度发 phase=`extract` | 同上 |
| 8 | `rename` 到 `core\<ver>\`，再清理上一个版本 | `E_STORE_IO` |
| 9 | `mihomo.exe -v` 冒烟测试，解析版本串比对；成功后发 phase=`done` | `E_CORE_START_FAILED` |

> 进度事件契约（C.7）：`core:install-progress` 负载
> `{version, phase, received, total, percent}`，`phase ∈ download|verify|extract|done`，
> 由 `fetch_core.rs` 内单一节流器发出（**≤2 条/秒**，比 `speedtest:progress` 更宽松，
> 因为下载耗时以秒计）。`done` 必须发一次，即使 `total` 未知（此时 `percent` 为 `null`）。

### 08.2.2 SHA256 校验的三种来源（优先级从高到低）

| 优先级 | 来源 | 可靠性 | 说明 |
| --- | --- | --- | --- |
| 1 | **代码内钉死的期望值**（`core_checksums.rs` 常量表） | 最高 | 发布新版本时人工核对后写入；断网也可校验 |
| 2 | 官方 Release 附带的校验文件（若该 Release 提供） | 高 | 文件名与格式随 Release 变化，**必须运行时发现**，解析失败即降级 |
| 3 | 仅校验 zip 结构 + `mihomo.exe -v` 版本串 | 兜底 | 只能防「下错资产」，防不了上游被篡改 |

> `TODO(待确认)`：mihomo v1.19.x 的 Release 是否稳定附带 `sha256sum.txt` 类文件需在 P1 实测确认。
> 在确认前，**以来源 1 为准**（每次升级内核版本都是一次人工核对点），来源 2 仅作二次校验。

```rust
// kernel/core_checksums.rs —— 升级内核时唯一需要改的表
pub const KNOWN_SHA256: &[(&str, &str)] = &[
    // (版本, compatible 资产 zip 的 sha256)
    ("v1.19.33", "0000000000000000000000000000000000000000000000000000000000000000"), // TODO 首次发布前填入实测值
];

pub fn expected_sha256(version: &str) -> Option<&'static str> {
    KNOWN_SHA256.iter().find(|(v, _)| *v == version).map(|(_, h)| *h)
        .filter(|h| !h.starts_with("0000")) // 占位值视为未知，强制走降级路径
}
```

### 08.2.3 下载实现（reqwest + tokio，含续传）

```rust
// kernel/fetch_core.rs
use std::path::{Path, PathBuf};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// C.7 `core:install-progress` 的负载，字段名照抄契约
#[derive(Clone, serde::Serialize)]
pub struct InstallProgress {
    pub version: String,
    pub phase: InstallPhase,      // download | verify | extract | done
    pub received: u64,
    pub total: Option<u64>,
    pub percent: Option<f32>,     // total 未知时为 None（前端显示不确定进度条）
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallPhase { Download, Verify, Extract, Done }

pub struct InstallCtx<'a> {
    pub version: String,
    pub client: &'a reqwest::Client,   // 由 util::net::build_client() 统一构造（UA/超时/代理）
    pub part: PathBuf,
    /// 内部负责 ≤2 条/秒节流（C.7），调用方只管发
    pub progress: &'a ProgressSink,
}

/// 节流器：同一 phase 内最多每 500ms 发一次；phase 切换与 done 强制发
pub struct ProgressSink {
    tx: tokio::sync::mpsc::Sender<InstallProgress>,
    last: std::sync::Mutex<(std::time::Instant, Option<InstallPhase>)>,
}

impl ProgressSink {
    pub fn emit(&self, p: InstallProgress) {
        let now = std::time::Instant::now();
        let mut guard = match self.last.lock() { Ok(g) => g, Err(_) => return };
        let (at, phase) = *guard;
        let forced = phase != Some(p.phase) || matches!(p.phase, InstallPhase::Done);
        if !forced && now.duration_since(at).as_millis() < 500 { return; }
        *guard = (now, Some(p.phase));
        drop(guard);
        let _ = self.tx.try_send(p);   // 满了就丢：进度事件允许丢帧
    }
}

/// 断点续传元数据，与 .part 同生命周期
#[derive(serde::Serialize, serde::Deserialize)]
struct PartMeta { version: String, url: String, total: Option<u64>, etag: Option<String> }

pub async fn download_zip(ctx: &InstallCtx<'_>, url: &str) -> Result<PathBuf, AppError> {
    // 1) 读续传元数据；URL 或版本变化则丢弃旧 .part（防止拼错文件）
    let meta_path = ctx.part.with_extension("zip.part.json");
    let resumed = tokio::fs::read(&meta_path).await.ok()
        .and_then(|b| serde_json::from_slice::<PartMeta>(&b).ok())
        .filter(|m| m.version == ctx.version && m.url == url);

    let mut have = match &resumed {
        Some(_) => tokio::fs::metadata(&ctx.part).await.map(|m| m.len()).unwrap_or(0),
        None => 0,
    };
    if resumed.is_none() { let _ = tokio::fs::remove_file(&ctx.part).await; }

    let mut req = ctx.client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity");
    if have > 0 { req = req.header(reqwest::header::RANGE, format!("bytes={have}-")); }
    if let Some(etag) = resumed.as_ref().and_then(|m| m.etag.clone()) {
        req = req.header(reqwest::header::IF_RANGE, etag);
    }

    let resp = req.send().await.map_err(AppError::from_reqwest)?; // 见 08.2.4 映射
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        let _ = tokio::fs::remove_file(&ctx.part).await;
        return Err(AppError::new(ErrCode::StoreIo, "续传位置无效，已重置下载"));
    }
    if !status.is_success() {
        return Err(AppError::http_status(status.as_u16()));
    }
    let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if !is_partial { have = 0; } // 服务器不支持 Range，从零写

    let total = resp.content_length().map(|l| l + have).or(resumed.as_ref().and_then(|m| m.total));
    // 2) 体积上限保护：内核 zip 正常 < 40 MB
    if let Some(t) = total { if t > 80 * 1024 * 1024 {
        return Err(AppError::new(ErrCode::CoreStartFailed, "内核包体积异常，已中止下载"));
    }}

    let mut file = tokio::fs::OpenOptions::new()
        .create(true).append(is_partial).write(true).truncate(!is_partial)
        .open(&ctx.part).await.map_err(AppError::from_io)?;

    let mut hasher = Sha256::new();
    if is_partial && have > 0 {
        // 续传时必须先对已有内容重算哈希（避免只校验后半段）
        let mut f = tokio::io::BufReader::new(tokio::fs::File::open(&ctx.part).await?);
        let mut buf = vec![0u8; 64 * 1024];
        use tokio::io::AsyncReadExt;
        loop {
            let n = f.read(&mut buf).await.map_err(AppError::from_io)?;
            if n == 0 { break; }
            hasher.update(&buf[..n]);
        }
    }

    let mut stream = resp.bytes_stream();
    let mut done = have;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(AppError::from_reqwest)?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(AppError::from_io)?;
        done += chunk.len() as u64;
        ctx.progress.emit(InstallProgress {
            version: ctx.version.clone(),
            phase: InstallPhase::Download,
            received: done,
            total,
            percent: total.map(|t| if t == 0 { 0.0 } else { (done as f32 / t as f32) * 100.0 }),
        });
    }
    file.flush().await.map_err(AppError::from_io)?;

    let meta = PartMeta { version: ctx.version.clone(), url: url.to_string(), total, etag: None };
    tokio::fs::write(&meta_path, serde_json::to_vec(&meta).map_err(AppError::from_json)?).await
        .map_err(AppError::from_io)?;

    // 3) 校验后 rename 成正式 zip，元数据随后删除
    ctx.progress.emit(InstallProgress {
        version: ctx.version.clone(), phase: InstallPhase::Verify,
        received: done, total, percent: None,
    });
    if let Some(expect) = expected_sha256(&ctx.version) {
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(expect) {
            let _ = tokio::fs::remove_file(&ctx.part).await;
            return Err(AppError::new(ErrCode::CoreStartFailed, "内核文件校验失败，已删除"))
                .with_detail(format!("sha256 mismatch, expect={expect} actual={actual}"));
        }
    } else {
        tracing::warn!(target: "kernel", "no pinned sha256 for version={}", ctx.version);
    }
    let zip = ctx.part.with_extension("zip");
    tokio::fs::rename(&ctx.part, &zip).await.map_err(AppError::from_io)?;
    let _ = tokio::fs::remove_file(&meta_path).await;
    Ok(zip)
}
```

### 08.2.4 解压与原子落位（防路径穿越）

```rust
pub fn extract_exe(zip: &Path, staging: &Path) -> Result<PathBuf, AppError> {
    let f = fs::File::open(zip).map_err(AppError::from_io)?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| AppError::zip(e.to_string()))?;
    fs::create_dir_all(staging).map_err(AppError::from_io)?;
    for i in 0..ar.len() {
        let mut e = ar.by_index(i).map_err(|e| AppError::zip(e.to_string()))?;
        // enclosed_name() 拒绝 "../" 与绝对路径 —— 必须使用，不可手写 strip
        let Some(rel) = e.enclosed_name() else { continue };
        if rel.file_name().and_then(|s| s.to_str()) != Some("mihomo.exe") { continue; }
        let out = staging.join("mihomo.exe");
        let mut w = fs::File::create(&out).map_err(AppError::from_io)?;
        std::io::copy(&mut e, &mut w).map_err(AppError::from_io)?;
        return Ok(out);
    }
    Err(AppError::new(ErrCode::CoreStartFailed, "内核包内未找到 mihomo.exe"))
}

/// 目录级 rename：同卷（都在 %LOCALAPPDATA%）时是原子的
pub fn commit_core(staging_exe: &Path, core_root: &Path, version: &str) -> Result<PathBuf, AppError> {
    let dst_dir = core_root.join(version);
    if dst_dir.exists() { fs::remove_dir_all(&dst_dir).map_err(AppError::from_io)?; }
    fs::create_dir_all(&dst_dir).map_err(AppError::from_io)?;
    let dst = dst_dir.join("mihomo.exe");
    // 跨目录同卷：rename 文件即可；staging 目录随后整删
    fs::rename(staging_exe, &dst).map_err(AppError::from_io)?;
    Ok(dst)
}
```

### 08.2.5 下载用的代理与网络参数

| 场景 | 取值 | 说明 |
| --- | --- | --- |
| 下载内核走代理 | `settings.core_download_proxy`（如 `http://127.0.0.1:1886`） | `Option<String>`；为空则直连 |
| 连接超时 | 8s（`connect_timeout`） | 与订阅拉取的 15s 分开配置 |
| 整体超时 | 300s（`Client::timeout`） | 内核 zip 约 10~16 MB |
| UA | `clash-zhengl/<version> (+kernel-download)` | GitHub 需要 UA |
| 重试 | 最多 2 次，间隔 1s/2s；**仅对连接类错误重试** | SHA 不匹配不重试（重试也是错的包） |
| 代理不可用 | `E_NET_PROXY` → UI 提示「下载代理 127.0.0.1:1886 不可用，请改直连」 | 见 C.8 |

> **构建期禁止联网**（00-README §4.5）：CI 不下载内核；内核只在**运行时**由 `core_install` 获取。

---

## 08.3 进程启动参数（唯一真源）

内核由 `supervisor` 以一个 `LaunchSpec` 单点生成**命令行 + runtime 配置**，两处数值绝不允许分叉：

```rust
pub struct LaunchSpec {
    pub exe: PathBuf,          // core\<ver>\mihomo.exe
    pub home: PathBuf,         // %LOCALAPPDATA%\ClashZhengl\runtime
    pub config: PathBuf,       // runtime\config.yaml
    pub mixed_port: u16,
    pub api_port: u16,
    pub secret: String,        // 32 hex
}

impl LaunchSpec {
    pub fn args(&self) -> Vec<String> {
        vec![
            "-d".into(), self.home.display().to_string(),
            "-f".into(), self.config.display().to_string(),
            "-secret".into(), self.secret.clone(),
            "-ext-ctl".into(), format!("127.0.0.1:{}", self.api_port),
        ]
    }
}
```

| 参数 | 值 | 必填 | 说明 |
| --- | --- | --- | --- |
| `-d <home>` | `%LOCALAPPDATA%\ClashZhengl\runtime` | ✅ | mihomo 的工作目录：`cache.db`、`geodata` 落此处；**启动前必须存在** |
| `-f <config>` | `runtime\config.yaml` | ✅ | 04 章产出的合并配置（原子写，见 C.3） |
| `-secret <s>` | 32 hex | ✅ | 与 `runtime\secret` 文件、runtime config 内 `secret` 完全一致 |
| `-ext-ctl <addr>` | `127.0.0.1:<port>` | ✅ | **禁止** `0.0.0.0`（C.12.2）；端口由 08.4 分配 |
| `-t` | — | 仅测试 | `core_install` 后、以及每次重建配置后跑一次「配置语法自检」，不启动服务 |
| `-v` | — | 仅探测 | 输出形如 `Mihomo Meta v1.19.33 windows amd64 with go1.24.x ...`，用于版本核对 |

**一致性规则**：runtime config 里**同时**写 `external-controller: 127.0.0.1:<api_port>` 与
`secret: <s>`（04 章要求），命令行再冗余传一次。二者由同一个 `LaunchSpec` 产生，因此天然一致；
若未来出现不一致，**mihomo 以命令行为准**，但这种情况属于 bug，必须在 code review 中拦下。

---

## 08.4 端口分配（避免 7890 冲突）

### 08.4.1 算法

```
allocate_pair(want_api: u16, reserved: &[u16]) -> Result<(mixed, api), AppError>
  1. 若 want_api == 0：api = random_high_port()；否则 api = want_api
  2. mixed = random_high_port()
  3. 各随机 32 次尝试：
       - 跳过 reserved 黑名单（见下表）
       - 跳过 mixed == api
       - TcpListener::bind("127.0.0.1:<p>") 成功即通过（立即 drop）
  4. want_api != 0 且被占用 → E_CORE_PORT_EXHAUSTED（显式端口冲突必须报错，不得静默换端口）
  5. 全部失败 → E_CORE_PORT_EXHAUSTED
```

```rust
// kernel/ports.rs
const RESERVED: &[u16] = &[80, 443, 1080, 3128, 3389, 7890, 7891, 7892, 7897, 8080, 8118, 8888, 9090, 10808];
const RANGE: std::ops::RangeInclusive<u16> = 40_000..=60_000;

fn probe_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).map(|l| drop(l)).is_ok()
}

pub fn allocate_pair(want_api: u16) -> Result<(u16, u16), AppError> {
    let mut rng = fastrand::Rng::new();
    let api = if want_api == 0 {
        (0..32).map(|_| rng.u16(RANGE)).find(|p| !RESERVED.contains(p) && probe_free(*p))
    } else {
        Some(want_api).filter(|p| probe_free(*p)) // 显式端口：占用了就失败，不偷偷换
    }.ok_or_else(|| AppError::new(ErrCode::CorePortExhausted, "无可用本地端口"))?;

    let mixed = (0..32).map(|_| rng.u16(RANGE))
        .find(|p| *p != api && !RESERVED.contains(p) && probe_free(*p))
        .ok_or_else(|| AppError::new(ErrCode::CorePortExhausted, "无可用本地端口"))?;
    Ok((mixed, api))
}
```

### 08.4.2 已知局限（必须在 P1 实测验证）

| 局限 | 后果 | 缓解 |
| --- | --- | --- |
| `bind` 探测后到 mihomo 真正 bind 之间有 TOCTOU 窗口 | 极小概率启动失败 | 启动后健康检查失败 → 记 `E_CORE_START_FAILED`，重试时换一组端口（最多 2 次） |
| Windows 上其他进程以 `SO_REUSEADDR` 占用时探测可能"成功" | 同上 | 同上，且错误提示给出端口号（**端口可进日志，secret 不可**，见 C.12.2 反面） |
| 用户 `settings.api_port` 显式指定且冲突 | `E_CORE_PORT_EXHAUSTED` | UI 提示「端口 9090 已被占用，请改为自动分配或换端口」 |

---

## 08.5 secret 生成与落盘

```rust
pub fn new_secret() -> String {
    let mut b = [0u8; 16];                       // 16 字节 => 32 hex
    getrandom::fill(&mut b).expect("os rng unavailable"); // 启动期不可恢复错误，允许 expect
    hex::encode(b)
}

/// 每次 core_start 都重新生成；写入 runtime\secret（用户私有目录，ACL 仅当前用户）
pub fn write_secret(home: &Path, secret: &str) -> Result<(), AppError> {
    let p = home.join("secret");
    let tmp = home.join("secret.tmp");
    fs::write(&tmp, secret.as_bytes()).map_err(AppError::from_io)?;
    fs::rename(&tmp, &p).map_err(AppError::from_io)?;   // 原子写，同 C.3
    Ok(())
}
```

| 规则 | 要求 |
| --- | --- |
| 长度 | 32 hex（16 字节 CSPRNG） |
| 生命周期 | **每次 `core_start` 重新生成**（C.12.2）；不复用旧 secret |
| 日志 | 任何级别、任何字段都不得出现 secret；`api.rs` 的 request builder 用 `HeaderValue::from_str` 直接构造，不打印 |
| 前端 | 通过 `RuntimeState.api_endpoint` 只知道地址，**永远拿不到 secret**；所有内核 API 调用都在 Rust 侧 |
| 诊断包 | `diagnostics_export` 必须把 `secret` 字段与 `Authorization` 头替换为 `******` |

---

## 08.6 状态机与健康检查

### 08.6.1 状态机（C.5 `CoreState`）

```
                 core_start
   ┌─────────┐  ────────────►  ┌──────────┐  /version OK   ┌─────────┐
   │ Stopped │                 │ Starting │ ─────────────► │ Running │
   └─────────┘ ◄────────────┐  └──────────┘                └─────────┘
        ▲                    │        │ 3s 内未就绪              │
        │  core_stop         │        ▼                          │ 进程退出(非预期)
        │              ┌──────────┐  ┌────────┐                 │
        └──────────────│ Stopping │  │ Failed │◄──── 5 次重试耗尽 │
                       └──────────┘  └────────┘                 │
                            ▲              │                    │
                            └──────────────┴────────────────────┘
                                    退避后重启(1s/2s/4s/8s)
```

每次迁移**必须** `emit("core:state", state)`（C.7），并在 `tracing` 打一行 `INFO [kernel] state ...`。

### 08.6.2 健康检查

```rust
// kernel/api.rs
pub struct CoreApi { base: String, secret: String, http: reqwest::Client }

impl CoreApi {
    pub async fn version(&self) -> Result<String, AppError> {
        let v: serde_json::Value = self.http
            .get(format!("{}/version", self.base))       // base = "http://127.0.0.1:<api_port>"
            .bearer_auth(&self.secret)                    // Authorization: Bearer <32hex>
            .timeout(Duration::from_millis(800))
            .send().await.map_err(AppError::from_reqwest)?
            .error_for_status().map_err(|e| AppError::core_api(e))?
            .json().await.map_err(AppError::from_reqwest)?;
        // 不同版本字段可能有差异，宽松取值，不要假设结构
        Ok(v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string())
    }
    pub async fn configs(&self) -> Result<serde_json::Value, AppError> { /* GET /configs 同上 */ }
}

/// 就绪探测：10 次 × 300ms = 3s 窗口，与 C.11「含内核代理可用 ≤3s」对齐
pub async fn wait_ready(api: &CoreApi, child: &mut tokio::process::Child) -> Result<(), AppError> {
    for attempt in 1..=10 {
        // 内核若已退出，立即失败，不要白等 3s
        if let Ok(Some(st)) = child.try_wait() {
            return Err(AppError::new(ErrCode::CoreStartFailed, "内核进程启动后立即退出")
                .with_detail(format!("exit_code={:?}", st.code())));
        }
        match api.version().await {
            Ok(v) if !v.is_empty() => {
                tracing::info!(target: "kernel", "ready version={} attempt={}", v, attempt);
                return Ok(());
            }
            Ok(_) | Err(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    Err(AppError::new(ErrCode::CoreStartFailed, "内核启动超时（3 秒内未响应）"))
}
```

| 探测项 | 端点 | 期望 | 失败映射 |
| --- | --- | --- | --- |
| 进程存活 | `child.try_wait()` | `None` | `E_CORE_START_FAILED`（带 exit code） |
| API 鉴权+就绪 | `GET /version` | 200 且 `version` 非空 | 401 → `E_CORE_API_UNAUTH`；其他 → 重试 |
| 配置生效 | `GET /configs` | 200，`mixed-port` 与期望一致 | `E_CORE_START_FAILED` |
| 混合端口可用 | 由前端/首次请求体现 | — | 记 `WARN`，不阻塞启动 |

### 08.6.3 `SelectorApi`：给 07 章用的抽象边界（C.6.1 冻结面 `v1.4-freeze-1`）

07 章的切换策略（`speedtest/selector.rs`）**只依赖这个 trait，不直接依赖 reqwest**。
这样 07 章可以用打桩实现做纯逻辑单测，也保证所有 HTTP 细节（Bearer、超时、错误归一）
只存在于 `kernel/api.rs` 一处。

> **本小节的签名是 C.6.1 的契约冻结面，各章必须逐字一致**（冻结版本 `v1.4-freeze-1`）。
> 任何改动都要同步 C.6.1 与 07 §7.16 并升冻结版本号。
>
> **v1.4 变更**：① trait 改为 **boxed-future 形式**（`ApiFuture`）以保证 dyn 兼容，
> 07 章可用 `Arc<dyn SelectorApi>` 注入 `FakeApi`；② `GroupInfo` / `SelectableGroup`
> 按 C.6.1 补齐 `test_url` / `hidden` / `now` 字段。v1.2 的「两版 `ApiError` 冲突」已闭合
> （core-writer 已把 07 章迁到五变体版），故删除原冲突说明。

```rust
// kernel/api.rs —— 07 章 selector 的唯一依赖面（C.6.1 冻结面）
use std::future::Future;
use std::pin::Pin;

/// dyn 兼容的装箱 Future。**不用 `async-trait` 宏**：它等价于 boxed future 但多一层宏依赖，
/// 本项目拒绝为其扩大依赖白名单（01 §5.5「不自造依赖」）。
pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

/// 可被切换的 select 组。`name` 为组名（`PUT /proxies/{group}` 的 {group}）。
/// **字段名是 `members`，不是 `candidates`**；不含 `hidden`——隐藏组在
/// `proxies_selectable` 里就被过滤掉，不会出现在返回值中。
pub struct SelectableGroup {
    pub name: String,
    pub members: Vec<String>,
    pub now: Option<String>,     // 该组当前选中节点
}

/// 单组当前状态。对应 mihomo `GET /proxies/{group}`：{ name, type, now, all, testUrl, hidden }
pub struct GroupInfo {
    pub name: String,
    /// ⚠️ 必须是**内核 API 原值**（大驼峰 `"Selector"`），不是 YAML 的 `select`。
    /// 比较须大小写敏感精确匹配，禁止 `contains("select")` 或 `to_lowercase() == "select"`
    /// —— `"Selector"` 转小写是 `"selector"`，与 YAML 的 `"select"` 并不相等。
    pub kind: String,
    pub now: Option<String>,
    pub all: Vec<String>,
    pub test_url: Option<String>, // API 的 testUrl 字段，可能为 None
    pub hidden: bool,
}

pub trait SelectorApi: Send + Sync {
    /// 可被切换的 select 组及其成员（已过滤 `hidden`）
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>>;
    /// 单组当前状态（name/type/now/all/test_url/hidden）
    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>;
    /// PUT /proxies/{group} {"name": node}
    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()>;
}

/// 内核 API 的统一错误。**不携带 reqwest 类型**，否则 trait 就泄漏了实现细节。
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,                 // → E_CORE_API_UNAUTH
    #[error("not found: {0}")]
    NotFound(String),             // 404：组/节点不存在
    #[error("timeout")]
    Timeout,                      // → E_NET_TIMEOUT（读超时）
    #[error("status {0}: {1}")]
    Status(u16, String),          // 其它非 2xx
    #[error("transport: {0}")]
    Transport(String),            // 连接失败：内核可能已退出
}

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        match e {
            // 401：secret 不一致（几乎只可能是 config 与命令行不同步）
            ApiError::Unauthorized =>
                AppError::new(ErrCode::CoreApiUnauth, "内核 API 鉴权失败（secret 不一致）"),
            // 超时按网络类错误处理，retryable
            ApiError::Timeout =>
                AppError::new(ErrCode::NetTimeout, "内核响应超时").retryable(true),
            // 404 与 5xx、连接失败都归到「内核启动/运行失败」，detail 带原始信息供排障
            ApiError::NotFound(m) =>
                AppError::new(ErrCode::CoreStartFailed, "内核 API 未找到目标").with_detail(m),
            ApiError::Status(code, body) =>
                AppError::new(ErrCode::CoreStartFailed, "内核 API 返回异常状态")
                    .with_detail(format!("status={code} body={}", truncate(&body, 256))),
            ApiError::Transport(m) =>
                AppError::new(ErrCode::CoreStartFailed, "无法连接内核，可能已退出")
                    .with_detail(m),
        }
    }
}
```

| `ApiError` | `AppError.code` | `retryable` | 典型场景 |
| --- | --- | --- | --- |
| `Unauthorized` | `E_CORE_API_UNAUTH` | false | secret 不匹配（应重启内核修复） |
| `Timeout` | `E_NET_TIMEOUT` | **true** | 内核过载 / 探测 URL 慢 |
| `NotFound` | `E_CORE_START_FAILED` | false | 组名写错、节点已被订阅更新删除 |
| `Status(u16, _)` | `E_CORE_START_FAILED` | `5xx` 时 true | 内核内部错误 |
| `Transport` | `E_CORE_START_FAILED` | true | 内核崩溃 / 端口被占 |

#### 实现侧写法（boxed-future：`Box::pin(async move { … })`）

trait 方法不再带 `async` 关键字，实现时要**自己装箱**。调用侧完全不受影响（照常 `.await`）：

```rust
// kernel/api.rs —— CoreApi 实现 SelectorApi
impl SelectorApi for HttpKernelApi {
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>> {
        Box::pin(async move {
            // GET /proxies → 只保留 API type == "Selector" 且 !hidden 的组
            let all: ProxiesResp = self.get_json("/proxies").await?;
            Ok(all.proxies.into_iter()
                .filter(|(_, p)| p.kind == "Selector" && !p.hidden)   // 精确匹配，禁止 to_lowercase
                .map(|(name, p)| SelectableGroup {
                    now: p.now.clone(),
                    members: p.all.clone(),
                    name,
                })
                .collect())
        })
    }

    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo> {
        let path = format!("/proxies/{group}");
        Box::pin(async move { Ok(self.get_json::<GroupInfo>(&path).await?) })
    }

    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()> {
        let path = format!("/proxies/{group}");
        let body = serde_json::json!({ "name": node });
        Box::pin(async move { self.put_json(&path, &body).await })   // 期望 204
    }
}
```

| 要点 | 说明 |
| --- | --- |
| 捕获变量的所有权 | `Box::pin(async move {})` 会移动捕获，所以 `format!` 出的 `path` 要在闭包**外**算好再 move 进去 |
| 借用 `&self` / `&str` | 生命周期由 `ApiFuture<'_, T>` 的匿名生命周期承接，与 `async fn` 的借用规则一致 |
| 成本 | 每请求一次装箱（一次堆分配），相对一次 HTTP 往返可忽略 |
| 为什么不用 `async-trait` | 该宏等价于 boxed future，但引入额外宏依赖；C.6.1 明令不扩大依赖白名单 |

**`reqwest` 错误的归一（`impl CoreApi` 内部）**：所有 `send()/json()` 的 `reqwest::Error`
先收敛成 `ApiError`，再转 `AppError`——**禁止**在 `selector.rs` 或 `commands/` 里见到
`reqwest::Error`：

```rust
fn map_reqwest(e: reqwest::Error) -> ApiError {
    if e.is_timeout() { return ApiError::Timeout; }
    if let Some(st) = e.status() {
        if st == reqwest::StatusCode::UNAUTHORIZED { return ApiError::Unauthorized; }
        if st == reqwest::StatusCode::NOT_FOUND { return ApiError::NotFound(e.url().map(|u| u.path().to_string()).unwrap_or_default()); }
        return ApiError::Status(st.as_u16(), String::new());
    }
    ApiError::Transport(e.to_string())
}
```

> **测试约定**：07 章单测用 `MockSelectorApi`（`HashMap<String, GroupInfo>` + 一个
> `Vec<(String, String)>` 记录调用），**不启内核、不发网络**。注入方式必须是
> `Arc<dyn SelectorApi>`——这正是本 trait 要用 boxed-future 的原因（裸 `async fn in trait`
> 在 Rust 1.85 下不是 dyn 兼容，`FakeApi` 放不进 `Arc<dyn ...>`）。
> 08 章的集成测试（12 章）才用真实 `CoreApi` 打在本地 mihomo 上。

---

## 08.7 进程监护：崩溃重启与退避

### 08.7.1 启动方式（隐藏窗口 + Job Object 兜底）

```rust
use std::os::windows::process::CommandExt;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

let mut cmd = tokio::process::Command::new(&spec.exe);
cmd.args(spec.args())
   .current_dir(&spec.home)
   .creation_flags(CREATE_NO_WINDOW)          // 0x08000000，绝不弹黑框
   .stdin(std::process::Stdio::null())
   .stdout(std::process::Stdio::piped())
   .stderr(std::process::Stdio::piped())
   .kill_on_drop(false);                      // 由 Job Object 统一兜底

let mut child = cmd.spawn().map_err(|e| AppError::new(
    ErrCode::CoreStartFailed, "内核启动失败")
    .with_detail(format!("spawn: {e}")))?;

// Job Object：本进程崩溃/被强杀时，内核随之终止，杜绝孤儿 mihomo 进程
let job = create_kill_on_close_job()?;
unsafe { AssignProcessToJobObject(job, HANDLE(child.raw_handle().unwrap() as _)) }
    .map_err(|e| AppError::new(ErrCode::CoreStartFailed, "无法托管内核进程")
        .with_detail(e.to_string()))?;
```

```rust
fn create_kill_on_close_job() -> Result<HANDLE, AppError> {
    unsafe {
        let job = CreateJobObjectW(None, None)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )?;
        Ok(job)
    }
}
```

> `windows` crate 的函数签名随版本有细微差异（`HINTERNET`/`HANDLE` 是否包 `Option`）。以
> `Cargo.lock` 里钉死版本的 rustdoc 为准，**不要凭记忆写**。本章代码以 windows 0.58 语义书写。

### 08.7.2 stdout/stderr 落盘与脱敏

```rust
async fn pump_log<R: tokio::io::AsyncRead + Unpin>(r: R, mut w: tokio::fs::File, tag: &'static str) {
    use tokio::io::AsyncBufReadExt;
    let mut lines = tokio::io::BufReader::new(r).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let safe = redact_core_line(&line);   // 见下
        let rec = format!("{} {} {}\n", now_rfc3339_local(), tag, safe);
        if w.write_all(rec.as_bytes()).await.is_err() { break; }
    }
}
```

`redact_core_line` 三条规则（C.9 要求「过滤内核 stdout 中的 secret」）：

| 模式 | 替换 |
| --- | --- |
| `secret[=: ]+[0-9a-fA-F]{32}` | `secret=******` |
| 含订阅 URL 的行（`token=`/`access_token=`） | 参数值 → `******` |
| 节点名 + `password`/`uuid` 断言 | 值 → `******` |

日志文件 `logs/core-YYYYMMDD.log`，单文件 2 MB × 保留 3 天（C.3）。

### 08.7.3 重启退避

| 尝试次数 | 等待 | 说明 |
| --- | --- | --- |
| 1（首次启动） | 0 | 不计入失败次数 |
| 第 2 次 | 1s | 非预期退出 |
| 第 3 次 | 2s | |
| 第 4 次 | 4s | |
| 第 5 次 | 8s | |
| 第 6 次 | — | **放弃**：`CoreState::Failed(reason)` + `notify` 事件 + 托盘图标变灰 |

```rust
fn backoff_delay(failed_attempts: u32) -> Duration {
    let ms = 1000u64.saturating_mul(1 << failed_attempts.min(3)); // 1s,2s,4s,8s
    Duration::from_millis(ms.min(8000))
}

/// 稳定性重置：连续运行 ≥60s 视为恢复，失败计数清零
const STABLE_AFTER: Duration = Duration::from_secs(60);
```

| 退出情形 | 处理 | 是否计入失败 |
| --- | --- | --- |
| 用户主动 `core_stop` | 正常，清理状态 | 否 |
| 应用退出流程 | 正常 | 否 |
| 配置错误导致启动即退（exit 1） | 重启**无意义**：首次失败后立刻 `-t` 校验配置并直接 `Failed(配置非法)` | 是，但**不重试** |
| 端口被抢占 | 换端口重试（最多 2 次），仍失败则 Failed | 是 |
| 被任务管理器/杀软杀掉 | 按退避重试 | 是 |
| 连续 5 次失败 | Failed + 通知「内核反复崩溃，请查看日志」 | 终止 |

---

## 08.8 优雅退出（先停内核，再退应用）

### 08.8.1 退出序列（顺序不可颠倒）

| 步 | 动作 | 超时 | 失败处理 |
| --- | --- | --- | --- |
| 1 | 停止调度器 tick 线程（取消 token） | 200ms | 忽略 |
| 2 | 取消进行中的测速/订阅任务 | 200ms | 忽略 |
| 3 | 若系统代理由本应用开启 → **恢复用户原值**（08.10） | 300ms | 记 WARN，继续 |
| 4 | 请求内核退出（Ctrl-Break 尽力而为） | 3s | 降级 TerminateProcess |
| 5 | `CloseHandle(job)`（KILL_ON_JOB_CLOSE 兜底） | — | — |
| 6 | flush 日志、写 journal `clean_exit=true` | 200ms | 忽略 |
| 7 | `app.exit(0)` | — | — |

### 08.8.2 代码骨架

```rust
// kernel/supervisor.rs
pub async fn stop_graceful(&self, child: &mut tokio::process::Child) -> Result<(), AppError> {
    let pid = child.id().ok_or_else(|| AppError::new(ErrCode::CoreStartFailed, "内核无 PID"))?;
    // Windows 没有 SIGTERM：向内核所在控制台发送 CTRL_BREAK。
    // Go 运行时把 CTRL_BREAK 映射为 os.Interrupt；mihomo 收到后走正常退出路径
    // （会 flush cache.db）。此行为在 P1 需实测确认，故另有 3s 兜底。
    let _ = tokio::task::spawn_blocking(move || send_ctrl_break(pid)).await;

    match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
        Ok(Ok(status)) => { tracing::info!(target: "kernel", "core exited code={:?}", status.code()); }
        _ => {
            tracing::warn!(target: "kernel", "core graceful stop timeout, terminating");
            let _ = child.kill().await;             // TerminateProcess
            let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
        }
    }
    Ok(())
}

/// 尽力而为的 Ctrl-Break：需要临时 AttachConsole 到子进程控制台
fn send_ctrl_break(pid: u32) {
    use windows::Win32::System::Console::{
        AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
        CTRL_BREAK_EVENT,
    };
    unsafe {
        let _ = FreeConsole();
        if AttachConsole(pid).is_ok() {
            let _ = SetConsoleCtrlHandler(None, true);          // 避免自己被打断
            let _ = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0);
            let _ = SetConsoleCtrlHandler(None, false);
            let _ = FreeConsole();
        }
    }
}
```

> **取舍记录**：mihomo 无「关闭」REST 端点，因此"优雅"只能是信号级尽力而为。
> 硬杀的唯一代价是 `cache.db` 可能丢失最近若干次选中状态；因我们设置了
> `profile.store-selected: true` 且 UI 侧保存 `RuntimeState.selected`，该代价可接受。

### 08.8.3 会触发退出的入口（全部走同一函数）

| 入口 | 处理 |
| --- | --- |
| 托盘「退出」 | `app.exit_requested = true; handle_exit()` |
| 主窗口关闭（`close_to_tray=false`） | 同上 |
| `core_stop` IPC | 只停内核，不退应用 |
| Windows 关机/注销（WM_QUERYENDSESSION） | 尽力执行 1–4 步，超时放弃（系统强制） |
| panic（`std::panic::set_hook`） | 恢复系统代理后 `std::process::abort()`；journal 保持 `clean_exit=false` 供下次守卫 |

---

## 08.9 Windows 系统代理（WinINET）

### 08.9.1 影响范围（必须先说清楚，避免用户误判）

| 程序类型 | 是否受 WinINET 系统代理影响 | 说明 |
| --- | --- | --- |
| Edge / Chrome / IE 内核浏览器 | ✅ | 读 WinINET 配置 |
| 使用 `WinINET`/`WinHTTP(自动)` 的 .NET 程序 | ✅ 多数 | `HttpClient` 默认走系统代理 |
| 显式读环境变量 `HTTP_PROXY` 的程序 | ❌ | 需用户手工设置 |
| 部分 UWP / 商店应用 | ❌ / 不确定 | 受 AppContainer 网络隔离影响 |
| Go 程序（含 mihomo 自身） | ❌ | `net/http` 默认只认环境变量 |
| 命令行工具（curl/git 等） | ❌ | 各工具自有配置 |

> 因此 **TUN 模式**是覆盖全局流量的唯一可靠手段（08.12），系统代理只是「够用且零驱动」的方案。

### 08.9.2 注册表目标（HKCU，无需管理员）

键：`HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings`

| 值名 | 类型 | 关闭状态 | 开启状态 | 说明 |
| --- | --- | --- | --- | --- |
| `ProxyEnable` | REG_DWORD | `0` | `1` | 总开关 |
| `ProxyServer` | REG_SZ | 恢复备份值/删除 | `127.0.0.1:<mixed_port>` | 单协议写法即可；HTTP/HTTPS 共用混合端口 |
| `ProxyOverride` | REG_SZ | 恢复备份值 | `<local>;localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.2*;172.30.*;172.31.*;192.168.*` | 内网直连，避免打不开路由器/内网系统 |
| `AutoConfigURL` | REG_SZ | 恢复备份值 | **临时删除** | PAC 优先级高于手动代理，不删则开关无效 |
| `Connections\DefaultConnectionSettings` | REG_BINARY | — | 由系统在 `Refresh` 后自行重写 | **不要手写这个 blob**（格式随系统版本变化），靠 `InternetSetOptionW` 触发系统刷新 |

### 08.9.3 刷新：`InternetSetOptionW`

注册表写完**不会自动生效**，必须通知系统重读：

```rust
use windows::core::PCWSTR;
use windows::Win32::Networking::WinInet::{
    InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
};

/// 顺序：SETTINGS_CHANGED(39) 通知配置已变 → REFRESH(37) 让 WinINET 重读
pub fn notify_proxy_changed() -> Result<(), AppError> {
    unsafe {
        let a = InternetSetOptionW(None, INTERNET_OPTION_SETTINGS_CHANGED, None, 0);
        let b = InternetSetOptionW(None, INTERNET_OPTION_REFRESH, None, 0);
        if a.0 == 0 || b.0 == 0 {
            return Err(AppError::new(ErrCode::StoreIo, "系统代理刷新失败")
                .with_detail(format!("GLE={}", windows::Win32::Foundation::GetLastError().0)));
        }
    }
    Ok(())
}
```

### 08.9.4 更可靠的写法：`INTERNET_OPTION_PER_CONNECTION_OPTION`

只写主键时，「设置 → 网络和 Internet → 代理」界面可能显示不一致，个别程序也只读 per-connection
blob。生产实现应**查询 → 修改 → 写回**（这也是 Clash Verge 一类项目的做法）：

```rust
// system/sysproxy.rs
use windows::Win32::Networking::WinInet::{
    InternetQueryOptionW, InternetSetOptionW,
    INTERNET_PER_CONN_FLAGS, INTERNET_PER_CONN_OPTIONW, INTERNET_PER_CONN_OPTIONW_0,
    INTERNET_PER_CONN_OPTION_LISTW, INTERNET_PER_CONN_PROXY_BYPASS,
    INTERNET_PER_CONN_PROXY_SERVER, INTERNET_OPTION_PER_CONNECTION_OPTION,
    PROXY_TYPE_DIRECT, PROXY_TYPE_PROXY,
};

/// 一个连接项的取值联合体；unsafe 是 WinINET 的 C ABI 决定的，封装在这里不扩散
fn set_conn_option(conn: Option<&str>, server: Option<&str>, bypass: Option<&str>) -> Result<(), AppError> {
    let mut conn_w: Vec<u16> = conn.map(|s| s.encode_utf16().chain([0]).collect()).unwrap_or_default();
    let mut srv_w:  Vec<u16> = server.map(|s| s.encode_utf16().chain([0]).collect()).unwrap_or_default();
    let mut byp_w:  Vec<u16> = bypass.map(|s| s.encode_utf16().chain([0]).collect()).unwrap_or_default();

    let mut opts: Vec<INTERNET_PER_CONN_OPTIONW> = Vec::with_capacity(3);
    opts.push(INTERNET_PER_CONN_OPTIONW {
        dwOption: INTERNET_PER_CONN_FLAGS,
        Value: INTERNET_PER_CONN_OPTIONW_0 {
            dwValue: if server.is_some() {
                PROXY_TYPE_DIRECT | PROXY_TYPE_PROXY
            } else {
                PROXY_TYPE_DIRECT
            } as u32,
        },
    });
    if server.is_some() {
        opts.push(INTERNET_PER_CONN_OPTIONW {
            dwOption: INTERNET_PER_CONN_PROXY_SERVER,
            Value: INTERNET_PER_CONN_OPTIONW_0 { pszValue: srv_w.as_mut_ptr() },
        });
    }
    if bypass.is_some() {
        opts.push(INTERNET_PER_CONN_OPTIONW {
            dwOption: INTERNET_PER_CONN_PROXY_BYPASS,
            Value: INTERNET_PER_CONN_OPTIONW_0 { pszValue: byp_w.as_mut_ptr() },
        });
    }

    let mut list = INTERNET_PER_CONN_OPTION_LISTW {
        dwSize: std::mem::size_of::<INTERNET_PER_CONN_OPTION_LISTW>() as u32,
        pszConnection: if conn_w.is_empty() { std::ptr::null_mut() } else { conn_w.as_mut_ptr() },
        dwOptionCount: opts.len() as u32,
        dwOptionError: 0,
        pOptions: opts.as_mut_ptr(),
    };

    let ok = unsafe {
        InternetSetOptionW(
            None,
            INTERNET_OPTION_PER_CONNECTION_OPTION,
            Some(&list as *const _ as *const core::ffi::c_void),
            list.dwSize,
        )
    };
    if ok.0 == 0 {
        return Err(AppError::new(ErrCode::StoreIo, "设置系统代理失败")
            .with_detail(format!("GLE={}", unsafe { windows::Win32::Foundation::GetLastError() }.0)));
    }
    Ok(())
}
```

> `INTERNET_PER_CONN_*` 的字段名与 `pszConnection` 的可空语义随 windows crate 版本有差异；
> 落地时以钉死版本 rustdoc 为准。**先用 `curl -x 127.0.0.1:<port> https://www.gstatic.com/generate_204`
> 验证端口已开，再开关系统代理**，否则只会得到「网页打不开」这一种现象。

### 08.9.5 开关状态机

```
        sysproxy_set{enabled:true}
IDLE ───────────────────────────► SNAPSHOT(读原值+写 journal)
  ▲                                     │
  │                                     ▼
  │                              APPLIED(写注册表 + 刷新)
  │                                     │ 校验：ProxyEnable==1 且 ProxyServer==本次端口
  │                                     ▼
  │                              FAILED(回滚 snapshot → notify)
  │
  └──── sysproxy_set{enabled:false} ── RESTORE(写回原值 + 刷新) ── 删除 journal
```

| 转换 | 前置校验 | 失败动作 |
| --- | --- | --- |
| IDLE→SNAPSHOT | 内核 `Running` 且 mixed 端口可连 | 返回 `E_CORE_START_FAILED`，提示「请先启动内核」 |
| SNAPSHOT→APPLIED | 读取 4 个注册表值全部成功（不存在也记 None） | `E_STORE_IO` |
| APPLIED 校验 | 回读比对 | 回滚 + `E_STORE_IO` |
| RESTORE | journal 存在则用 journal 的值；否则用当前进程快照 | 记 WARN + `notify` |

> **注意**：只有内核 Running 时才允许开启系统代理（否则开启后所有网页都打不开，用户会认为软件坏了）。
> `sysproxy_set` 必须做该校验并返回明确中文错误。

### 08.9.6 每次转换后必须回填 `SysProxyState` 并发事件

C.6 规定 `sysproxy_set` 的返回值是 **`SysProxyState`**（不再是 `{applied: bool}`），
C.7 规定状态变化要发 `sysproxy:state`。因此状态机的每个终态都要（a）回读注册表算出真值，
（b）写入 `RuntimeState.sysproxy`，（c）发事件：

```rust
// system/sysproxy.rs —— 唯一的状态生产函数：真值永远从注册表回读，不靠内存猜
pub fn read_state(expected_port: Option<u16>) -> Result<SysProxyState, AppError> {
    let enabled = reg_read_dword(HKCU_SETTINGS, "ProxyEnable")?.unwrap_or(0) != 0;
    let server = reg_read_string(HKCU_SETTINGS, "ProxyServer")?;   // 可能为 None
    // managed_by_us 判定：journal 存在且未 cleanExit，且 server 指向我们的回环端口
    let ours = server
        .as_deref()
        .and_then(parse_loopback_port)          // "127.0.0.1:41023" -> Some(41023)
        .filter(|p| Some(*p) == expected_port || is_our_port(*p));
    Ok(SysProxyState {
        enabled,
        server,
        managed_by_us: ours.is_some() && journal_exists_dirty(),
        port: ours,
    })
}

/// IPCsysproxy_set：任一分支都以 read_state() 的结果作为返回值
#[tauri::command]
pub async fn sysproxy_set(
    state: tauri::State<'_, AppState>, enabled: bool,
) -> Result<SysProxyState, AppError> {
    if enabled { state.sysproxy.enable().await? } else { state.sysproxy.disable().await? }
    let st = read_state(state.kernel.mixed_port())?;     // 回读真值
    state.runtime.write().sysproxy = st.clone();          // 同步进 RuntimeState
    state.emit("sysproxy:state", &st)?;                   // C.7 事件
    Ok(st)
}
```

| 时机 | 必须做的事 |
| --- | --- |
| 转换成功 | `read_state()` → 写 `RuntimeState.sysproxy` → `emit("sysproxy:state")` |
| 转换失败/回滚后 | **同样**要 `read_state()` + emit，让 UI 回到真实态（不能只抛错不刷状态） |
| 应用启动 | 启动守卫（08.10.2）跑完后调用一次 `read_state()`，作为 `app_bootstrap` 里 `runtime.sysproxy` 的初值 |
| 外部改动 | 每 30s 或窗口获得焦点时低频 `read_state()`；若与缓存不同 → 更新并发 `sysproxy:state`（C.7 明文「含被外部程序改动后探测到」） |

> **`managed_by_us` 的语义**（C.5 注释）：`true` = 该值由本应用写入且尚未恢复。
> 它**不是**「现在代理开着吗」——那是 `enabled`。UI 用它来决定「关闭代理」按钮是
> 「恢复你原来的设置」还是「关掉你自己开的代理」，措辞不同（见 09 章 09.8.1）。

---

## 08.10 崩溃恢复守卫（关键设计）

系统代理是**写在用户机器上的持久状态**。应用崩溃/被强杀后，如果代理还指着已死掉的本地端口，
用户的网络会完全瘫痪（所有网页打不开）。这是本类产品最恶劣的故障，必须专门设计。

### 08.10.1 journal 文件

`%LOCALAPPDATA%\ClashZhengl\runtime\sysproxy.journal.json`（原子写，崩溃最多丢一次写）

```jsonc
{
  "schemaVersion": 1,
  "ownerPid": 12345,
  "ownerExe": "C:\\Users\\me\\AppData\\Local\\ClashZhengl\\clash-zhengl.exe",
  "appVersion": "0.1.0",
  "enabledAt": 1762500000,
  "appliedProxyServer": "127.0.0.1:41023",
  "appliedBypass": "<local>;localhost;127.*;10.*;172.16.*;...",
  "removedAutoConfigUrl": "http://pac.example.com/proxy.pac",  // 若原本有 PAC，必须记住
  "snapshot": {
    "proxyEnable": 0,
    "proxyServer": "",
    "proxyOverride": "<local>",
    "autoConfigUrl": null
  },
  "cleanExit": false
}
```

写入时机：`APPLIED` 成功之后立刻 `cleanExit=false`；正常关闭系统代理或正常退出应用时
`cleanExit=true` 并删除文件。

### 08.10.2 启动时的守卫算法（`app.rs` → `ready` 阶段，BEFORE 启动内核）

```
read journal
├─ 不存在 或 cleanExit==true
│    → 删除文件，结束（正常情况）
└─ cleanExit==false（上次是崩溃/被杀）
     ├─ 读当前注册表 ProxyServer
     ├─ 不是 "127.0.0.1:<我们的端口>"（用户已手工改过/别的软件接管）
     │    → 只删除 journal，**不动注册表**（绝不覆盖用户的当前选择）
     └─ 是我们留下的
          ├─ 恢复 snapshot（ProxyEnable/Server/Override/AutoConfigURL 全部写回）
          ├─ 刷新 + 删除 journal
          ├─ 写 WARN 日志：`sysproxy leftover restored port=xxxxx`
          └─ emit `notify` {level:"warn", title:"已恢复系统代理",
                            body:"上次异常退出后系统代理未关闭，已自动恢复为你原来的设置"}
```

**为什么选「自动恢复 + 通知」而不是「弹窗询问」**：崩溃往往是用户没察觉的（例如更新中被强杀），
此时网络已全断，用户最需要的是「立刻能上网」而不是回答一个问题。通知只做告知，提供「查看日志」。

### 08.10.3 三重保险

| 保险 | 实现 | 覆盖场景 |
| --- | --- | --- |
| 1. 正常退出路径 | 退出序列第 3 步恢复 | 用户点退出/关机 |
| 2. panic hook | `panic::set_hook` 内同步恢复（禁止 async） | 主线程 panic |
| 3. 启动守卫 | 08.10.2 的 journal 检查 | 强杀、蓝屏、断电、Task Manager 结束进程 |

补充：**可选**的守护进程方案（不做，记录取舍）——另起一个极简 watchdog 进程，主进程消失后恢复代理。
不做的理由：额外进程 + 额外内存（约 3 MB）+ 杀不掉的风险，与轻量目标冲突；journal 方案已足够。

---

## 08.11 设置热应用矩阵（C.6 `settings_save` 引用本章）

`settings_save` 保存成功后按此表决定「立即生效 / 需重启内核 / 需重启应用」：

| 设置项 | 生效方式 | 触发动作 | 对用户提示 |
| --- | --- | --- | --- |
| `theme` / `locale` | 立即 | 前端重渲染 | 无 |
| `close_to_tray` | 立即 | 更新托盘行为 | 无 |
| `log_level` | 立即 | `tracing_subscriber` reload handle | 无 |
| `auto_launch` / `silent_start` | 立即 | 写/删 HKCU Run | 无 |
| `sub_default_interval_min` / `sub_fetch_*` | 立即 | 只影响后续拉取 | 无 |
| `speedtest_*` / `switch_policy` | 立即 | 调度器读新配置 | 无 |
| `api_port` | **需重启内核** | `core_restart`（可选自动） | 「端口已修改，需要重启内核生效」 |
| `core_version` | **需重启内核** | 先 `core_install` 再 `core_restart` | 「正在切换内核版本…」 |
| `proxy_mode` | **需重启内核** | `SystemProxy→Tun` 需提权，见 08.12 | 「切换代理模式需要重启内核」 |
| `bind_address` | **不开放修改** | 固定 `127.0.0.1`（C.12.2） | 只读展示 |

> 禁止在 `settings_save` 里同步重启内核（会让 IPC 卡住数秒）。正确做法：保存 → 立刻返回 →
> 后台任务重启 → 通过 `core:state` 事件推进度。

---

## 08.12 TUN 模式（可选能力，非 MVP 阻塞项）

| 项 | 要求 |
| --- | --- |
| 权限 | **必须管理员**；默认非管理员运行（C.12.4）。检测到非管理员时返回 `E_PERM_ADMIN` |
| 依赖 | mihomo 自带 `wintun.dll` 需求；需随内核一起落地（同 zip 内文件，解压时一并取出） |
| 配置 | `tun: { enable: true, stack: gvisor, auto-route: true, auto-detect-interface: true }` |
| 与系统代理关系 | 二选一；切 TUN 时应关闭系统代理（避免双重代理导致的自我环路） |
| 提权方式 | 「以管理员身份重启」按钮 → `ShellExecuteW(runas)` 重新拉起自身并传 `--elevated` |
| 失败表现 | 驱动签名被拦（杀软/组策略）→ `E_CORE_START_FAILED`，detail 给 mihomo stderr 末 5 行 |

> MVP 先只做系统代理；TUN 作为 P6 之后的可选项。文档与 UI 都必须明确「TUN 需要管理员权限」，
> 不允许悄悄提权。

---

## 08.13 开机自启与单实例

### 08.13.1 开机自启（HKCU Run，无需管理员）

| 项 | 值 |
| --- | --- |
| 键 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` |
| 值名 | `ClashZhengl` |
| 值数据 | `"<exe 绝对路径>" --autostart`（路径含空格必须带引号） |
| 关闭 | `RegDeleteValueW`；**只删自己写的那项**，删除前校验值数据含 `clash-zhengl.exe` |

| 备选方案 | 优点 | 缺点 | 是否采用 |
| --- | --- | --- | --- |
| HKCU Run | 零权限、可预测、易卸载清理 | 不支持提权 | ✅ **采用** |
| 计划任务（Task Scheduler） | 可「最高权限运行」 | 卸载残留、杀软更敏感 | ❌ |
| 启动文件夹快捷方式 | 简单 | 用户可见、易误删 | ❌ |

`--autostart` 参数作用：若 `settings.silent_start == true`，则**不显示窗口**，只进托盘
（C.5 `silent_start` 默认 true）。

### 08.13.2 单实例

```rust
// system/single_instance.rs
const MUTEX_NAME: &str = r"Local\ClashZhengl.SingleInstance.v1";
const EVENT_NAME: &str = r"Local\ClashZhengl.Activate.v1";

/// 返回 true 表示本进程是「第一个实例」
pub fn acquire() -> bool {
    unsafe {
        let mutex = CreateMutexW(None, true, w!(MUTEX_NAME));
        let err = GetLastError();
        if err == ERROR_ALREADY_EXISTS {
            // 通知已存在的实例显示窗口，然后自己退出
            if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!(EVENT_NAME)) {
                let _ = SetEvent(ev);
            }
            return false;
        }
        std::mem::forget(mutex); // 生命周期 = 进程生命周期
        true
    }
}
```

| 场景 | 行为 |
| --- | --- |
| 用户重复双击图标 | 第二实例退出，第一实例窗口 `show() + set_focus()` |
| 安装包升级时启动 | 同上；NSIS 安装前会先杀进程（见 11 章） |
| 会话隔离 | 用 `Local\` 前缀（非 `Global\`），多用户登录互不干扰 |

---

## 08.14 异常场景总表（演练清单，P1 必过）

| # | 场景 | 期望行为 | 用户可见 | 错误码 |
| --- | --- | --- | --- | --- |
| 1 | 内核未安装 | 不崩溃，UI 显示引导页 | 「未安装内核，请点击下载」 | `E_CORE_NOT_FOUND` |
| 2 | 下载中网络中断 | 保留 `.part`，可续传；提示重试 | 「下载失败，已保留进度，可重试」 | `E_NET_TIMEOUT` |
| 3 | SHA256 不匹配 | 删除残留文件，拒绝安装 | 「内核文件校验失败，已删除」 | `E_CORE_START_FAILED` |
| 4 | mixed 端口被其他程序占用 | 自动换端口重试 2 次 | 无感（日志可见） | — |
| 5 | 显式 `api_port` 被占用 | 不自动换，明确报错 | 「端口 9090 已被占用」 | `E_CORE_PORT_EXHAUSTED` |
| 6 | 配置 YAML 非法（用户手改） | 用 `-t` 定位，回退上一份可用配置 | 「配置已损坏，已回退到上次可用配置」 | `E_PARSE_YAML` |
| 7 | config 内 secret 与命令行不一致 | 由 `LaunchSpec` 保证不可能；若仍 401 | 「内核鉴权失败」 | `E_CORE_API_UNAUTH` |
| 8 | 内核启动后 1s 秒退 | 读 stderr，返回可读原因；不无限重试 | 「内核启动失败：<原因摘要>」 | `E_CORE_START_FAILED` |
| 9 | 任务管理器强杀内核 | 按 1/2/4/8s 退避重启，成功则无感 | 无（首次静默）；5 次后告警 | — |
| 10 | 杀软拦截 mihomo.exe | 启动失败/文件被隔离 | 「内核可能被安全软件拦截，请添加信任」 | `E_CORE_START_FAILED` |
| 11 | 应用被强杀时代理开着 | 下次启动自动恢复原代理 + 通知 | 通知气泡 | — |
| 12 | 用户手工改了系统代理 | 我们的开关回读为「关闭」，不覆盖用户值 | 开关自动置为关 | — |
| 13 | PAC 自动配置脚本存在 | 开启前备份并临时移除，关闭时恢复 | 无感（日志可见） | — |
| 14 | 磁盘满（`runtime` 写入失败） | 不崩溃，提示磁盘不足 | 「磁盘空间不足，无法写入配置」 | `E_STORE_IO` |
| 15 | 升级时旧内核仍占用文件 | 升级前先停内核；删除失败记 WARN 跳过 | 无感 | — |

> 每一项都要对应到 `12-testing-and-acceptance.md` 的手工验收清单，逐条打勾。

---

## 08.15 验收要点（本阶段门禁）

1. `core_install` → 下载 → 校验 → 解压 → `mihomo.exe -v` 版本串与预期一致；
2. `core_start` 后 `core_status` 为 `Running`，`GET /version` 通，混合端口可用
   （`curl -x http://127.0.0.1:<mixed> https://www.gstatic.com/generate_204` 返回 204）；
3. 强杀 `mihomo.exe` → 日志出现 `restart attempt=1 delay=1s`，5 秒内恢复 Running；
4. 连续 5 次失败后 `CoreState::Failed`，托盘变灰，通知弹出；
5. 开系统代理 → 系统设置界面显示一致（`INTERNET_OPTION_PER_CONNECTION_OPTION` 生效）→ 关代理后原值恢复；
6. 在开代理状态下用任务管理器杀掉 `clash-zhengl.exe` → 重启应用 → 通知「已恢复系统代理」→
   `ProxyEnable` 回到 0；
7. 退出应用耗时 ≤ 1.5s（含内核 3s 兜底不计入，正常路径应为 <500ms）；
8. `runtime\`、`logs\` 全文搜索无 `secret` 明文（`grep -i "secret=" logs/*` 只应出现 `******`）；
9. 单实例：双击第二次只让已有窗口前置，不产生第二个托盘图标。

---

## 08.16 契约对齐记录（v1.1）

> C 章于 v1.1 裁决了本章提出的 6 条缺口。下表记录**已落地**的对齐点；
> 原「冲突点」表随之作废（保留于 git 历史）。

| # | 裁决结果（C 章） | 本章修改 | 对应节 |
| --- | --- | --- | --- |
| 1 | `RuntimeState` 新增 `pub sysproxy: SysProxyState`；`SysProxyState { enabled, server, managed_by_us, port }` | 新增 **08.9.6**（`read_state()` 真值回读 + 每终态回填 `RuntimeState.sysproxy` + 发 `sysproxy:state`）；08.10.2 守卫改为写该字段；删除原「无字段」缺口 | C.5 `RuntimeState` / `SysProxyState`；C.7 `sysproxy:state` |
| 2 | 新事件 `core:install-progress {version, phase, received, total, percent}`，phase ∈ `download\|verify\|extract\|done`，≤2 条/秒 | 08.2.1 流程表逐步标注 phase；新增 `InstallProgress` / `InstallPhase` / `ProgressSink`（500ms 节流 + phase 切换与 done 强制发）；下载循环改发完整负载 | C.7 `core:install-progress` |
| 3 | C.3 新增 `runtime/download/`、`runtime/sysproxy.journal.json` | 08.1.2 目录树已用契约路径（`runtime\download\`、`runtime\sysproxy.journal.json`），本轮回读确认一致，无改名 | C.3 |
| 4 | `sysproxy_set` 返回值改为 `SysProxyState` | 08.9.6 给出命令实现：任一分支均 `read_state()` 后返回，并同步 `RuntimeState` + emit | C.6 `sysproxy_set` |
| 5 | 新增 `core_versions_list` → `{installed, current, available}`（available 失败返回空数组且不报错） | 08.1.2 `resolve_core_exe` 增补 `core_versions_list` 的构造方式（`current` 取 `settings.core_version` 或最新已装）；设置页版本下拉见 09 章 09.7 | C.6 `core_versions_list` |
| 6 | `TrafficSummary` 已在 C.5 完整定义 | 本章不含流量渲染逻辑，无需改动；引用关系见 09 章 09.5 | C.5 `TrafficSummary` |

### 08.16.1 顺带确认的三条路径口径

| 路径 | 契约来源 | 本章用法 |
| --- | --- | --- |
| `%LOCALAPPDATA%\ClashZhengl\runtime\download\` | C.3 | `.part` / `.part.json` / `extract-<ver>\` 均在此，启动成功后整目录删除 |
| `%LOCALAPPDATA%\ClashZhengl\runtime\sysproxy.journal.json` | C.3 | 08.10 守卫的唯一依据；`cleanExit` 标记在此 |
| `%LOCALAPPDATA%\ClashZhengl\runtime\core\<version>\` | C.3 | 版本目录名即版本号，切换版本只改 `settings.core_version` 指针 |
| `%LOCALAPPDATA%\ClashZhengl\runtime\switch-history.json` | C.3（v1.1） | **07 章**写入与裁剪（最近 200 条）；本章只负责 GC 时不误删（见 08.1.2 注） |

---

## 08.17 契约对齐记录（v1.2）

> C 章 v1.2 的 3 条裁决中，仅第 1 条（`switch_apply_best` 返回 `{primary, all}`）与第 2 条
> （`SwitchRecord.to_delay` → `Option<u32>`）**经由 07 章的 API 边界影响本章**；第 3 条
> （`manual_override_min`）是纯 UI 与 `speedtest/selector.rs` 的事，本章无改动。

| # | 裁决结果（C 章） | 本章修改 | 对应节 |
| --- | --- | --- | --- |
| 1 | **新增对接要求**：`kernel/api.rs` 必须提供 `SelectorApi` 三方法与统一 `ApiError`，供 07 章使用 | 新增 **08.6.3**：`SelectableGroup` / `GroupInfo` / `SelectorApi` / `ApiError` 五变体；另给 `From<ApiError> for AppError` 映射实现与 `map_reqwest()` 归一函数（v1.4：trait 形式已改为 `ApiFuture`，见 08.17.3） | C.6.1 `SelectorApi` / `ApiError`；07 §7.16 冻结表 |
| 2 | `SwitchRecord.to_delay: Option<u32>` | 本章不渲染历史记录，无需改动；`to_delay` 的消费方在 09 章 09.6.7（`fmtDelayPair`） | C.5 `SwitchRecord` |
| 3 | `manual_override_min` + 手动覆盖语义（`SwitchRuntimeLocks` 在 `speedtest/selector.rs`，不进 `RuntimeState`） | 本章无改动（覆盖期是 07 章内存态 + 09 章 UI）；仅确认**不**把 `SwitchRuntimeLocks` 加进 `RuntimeState` | C.5 手动覆盖语义 |
| 4 | C.3 新增 `runtime/switch-history.json` | 08.1.2 目录树补该文件并注明归属 07 章；明确 GC **不得**对整个 `runtime\` 做 `remove_dir_all` | C.3 |

### 08.17.1 `ApiError → AppError` 映射表（07 章按此上报错误）

| `ApiError` | `AppError.code` | `retryable` | 用户可见文案（C.8 原文） |
| --- | --- | --- | --- |
| `Unauthorized` | `E_CORE_API_UNAUTH` | `false` | 内核 API 鉴权失败（secret 不一致） |
| `NotFound(_)` | `E_CORE_START_FAILED` | `false` | 内核启动失败：{detail} |
| `Timeout` | `E_NET_TIMEOUT` | `true` | 连接订阅服务器超时 → 此处 detail 改写为「内核响应超时」 |
| `Status(u16, _)` | `E_CORE_START_FAILED` | `5xx` 时 `true` | 内核启动失败：{detail} |
| `Transport(_)` | `E_CORE_START_FAILED` | `true` | 无法连接内核，可能已退出 |

> **为什么 `Timeout` 映射到 `E_NET_TIMEOUT` 而不是 `E_CORE_START_FAILED`**：C.8 的
> `E_NET_TIMEOUT` 语义是「连接/读取超时」，内核 API 读超时正属此类，且其 `retryable=true`
> 语义正确（重试通常能成功）。而 `E_CORE_START_FAILED` 语义是「启动失败/端口占用」，
> 若把读超时也归进去，UI 会误报「内核启动失败」并暗示用户重启内核。
> **detail 字段**用于区分「是内核慢还是订阅服务器慢」，不得把两者混成一个码。

> **架构约束（重申）**：`speedtest/selector.rs`（07 章）**只** `use kernel::api::{SelectorApi, ApiError}`，
> 禁止 `use reqwest::*`。这条约束让 07 章的全部策略单测可以在**无网络、无内核**的情况下跑完，
> 也是 C.13「纯函数必须有单测」能落地的前提。

### 08.17.2 `ApiError` 版本冲突——已闭合（v1.4）

v1.2 落稿时曾发现 07 §7.6.1 的 `ApiError`（`Status(u16)/Connect/Decode/Other`）与本章采用的
五变体版（`Unauthorized/NotFound/Timeout/Status(u16,String)/Transport`）不一致。

**v1.4 结论：core-writer 已把 07 章迁移到五变体版**，两章现已一致，
冻结表版本 `v1.4-freeze-1`（随 `ApiFuture` 一起升级）。因此：

- 08.6.3 的 `ApiError` 定义与 08.17.1 的映射表**保持不变**，无需再改；
- 原「迁移映射表」已从 08.6.3 删除（07 章已迁完，映射无保留价值）；
- 本条仅作历史记录，**不再是待裁决项**。

> `GroupInfo` / `SelectableGroup` 的字段在此轮也按 C.6.1 补齐（`test_url`、`hidden`、`now`），
> 属**新增字段而非改名**，07 章实现侧若已按旧字段写，需补这两个字段的赋值。

### 08.17.3 v1.4：`SelectorApi` 改用 boxed-future（dyn 兼容）

C.6.1 定稿要求 trait 用 **boxed-future** 而非裸 `async fn`，原因是 Rust 1.85 下
`async fn in trait` **默认不是 dyn 兼容**的，而 07 章需要 `Arc<dyn SelectorApi>` 注入 `FakeApi`。

| 项 | v1.2（旧） | v1.4（现行） |
| --- | --- | --- |
| trait 方法 | `async fn f(&self) -> Result<T, ApiError>` | `fn f(&self) -> ApiFuture<'_, T>` |
| 宏依赖 | `#[async_trait]` | **无**（去掉 `async-trait` crate） |
| dyn 兼容 | ❌ 不能 `Arc<dyn SelectorApi>` | ✅ 可以 |
| 实现侧 | `async fn f(...)` | `Box::pin(async move { … })` |
| 调用侧 | `.await` | `.await`（**完全一致，无感知**） |
| 成本 | — | 每请求一次装箱 |

本章改动：08.6.3 的 trait 定义换成 `ApiFuture` 形式、删 `use async_trait::async_trait;`、
补 `impl SelectorApi for HttpKernelApi` 的实现侧示例与「捕获变量要在闭包外算好」的注意事项。
`ApiError` 五变体与 08.17.1 映射表**不变**。

### 08.17.4 v1.4：配置字段取值（本章无改动，交叉引用）

附录 A 修正了三处 mihomo v1.19.33 取值，**本章不生成合并配置**（那是 04 章职责），
故无代码改动；但健康检查与排障时需知道这几个值，避免误判：

| 字段 | 正确值 | 错值的后果 |
| --- | --- | --- |
| `find-process-mode` | `strict` | 写成 `off` 会使 `helper.FindProcess = nil`，机场 `PROCESS-NAME` 规则**永久静默失效**（无报错，规则只是不匹配） |
| `ipv6` | `true` | 写 false 会让纯 AAAA 的节点不可达 |
| `dns.enable` | `false`（SystemProxy 档） | 启用时 `updateDNS` 会真的建 DNS 服务；`!Enable` 时直接 `ReCreateServer("", nil, nil)` 不建服务 |

> 附录 A.6.2 模板里的 `url-test` 组已删除（C.5 组类型约束：`scope` 组必须是 `select`）。
> 08 章健康检查只看 `GET /configs` 的 `mixed-port`，不校验这些字段；
> 若启动后发现 `PROCESS` 规则不生效，**优先怀疑 `find-process-mode`** 而不是规则语法。

---

## 08.18 本章 TODO(待确认) 清单

> 以下均为**需实测确认的外部事实**，不是契约缺口。契约缺口已全部由 C v1.1 / v1.2 裁决。

| # | 待确认项 | 当前默认 | 确认方式 |
| --- | --- | --- | --- |
| 1 | mihomo Release 是否稳定附带校验和文件 | 视为「不附带」，用代码内钉死值 | 打开 v1.19.33 Release 页面核验 |
| 2 | `GenerateConsoleCtrlEvent(CTRL_BREAK)` 能否让 mihomo 优雅退出 | 视为「有效但需 3s 兜底」 | P1 实测：观察 `cache.db` mtime 是否更新 |
| 3 | `mihomo -v` 输出格式稳定性 | 用宽松正则 `v?(\d+\.\d+\.\d+)` 提取 | 实测 v1.19.33 |
| 4 | `-t` 在配置非法时的退出码 | 视为非 0 | 用故意改坏的 yaml 实测 |
| 5 | Win10 1809 上 `INTERNET_PER_CONN_OPTION` 行为与 Win11 是否一致 | 视为一致 | 双系统实测 |

> 已删除的 6 条契约类 TODO（`TrafficSummary` 未定义、`RuntimeState` 无 `sysproxy`、无下载进度事件、
> 无版本列表命令、C.3 无 `download/` 与 `journal` 路径、`sysproxy_set` 返回结构）见 08.16。

---

## 08.19 本章自检清单（Agent 必须逐条确认）

> 交付任何内核/系统代理代码前逐条回答「是/否」，任一「否」不得进入下一阶段。

- [ ] 1. 内核是作为**独立子进程**启动的吗？有没有静态链接、修改 mihomo 源码、或把它当库调用（01 §8 / C.12.5）？
- [ ] 2. `Command` 是否设了 `CREATE_NO_WINDOW`？是否挂进 Job Object（`KILL_ON_JOB_CLOSE`）防孤儿进程？
- [ ] 3. 启动参数是否**只**由 `LaunchSpec` 单点生成（`-d`/`-f`/`-secret`/`-ext-ctl`）？有无在别处手拼命令行？
- [ ] 4. `secret` 是否每次启动重新生成（32 hex）？是否**没有**出现在命令行回显、日志、`RuntimeState` 里（C.12.2）？
- [ ] 5. `external-controller` 是否绑定 `127.0.0.1`？有无出现 `0.0.0.0` 或空 secret？
- [ ] 6. 端口是否从 40000–60000 随机 + bind 探测？是否排除保留端口（7890/7891/9090/…）？
      **显式指定端口冲突时是否报错而非偷偷换端口**？
- [ ] 7. 下载是否校验 SHA256（三来源优先级：钉死值 > Release 校验文件 > `-v` 冒烟）？校验失败是否删除文件并报错？
- [ ] 8. 解压是否用 `zip::enclosed_name()` 防路径穿越？**有没有**手写 `strip_prefix` 之类的自制判断？
- [ ] 9. 解压落位是否「先 extract 到暂存目录 → 整体 `rename`」？断点续传是否在续传前**重算已有内容的 hash**？
- [ ] 10. 就绪探测是否 10×300ms（≤3s，C.11）？内核若提前退出是否立即失败而不再白等？
- [ ] 11. 崩溃重启退避是否为 1s/2s/4s/8s 且最多 5 次后转 `Failed`？是否每次都发 `core:state`？
- [ ] 12. 优雅退出是否走 `GenerateConsoleCtrlEvent(CTRL_BREAK)` + 3s 后 `TerminateProcess` 兜底？
- [ ] 13. 系统代理快照是否写 `runtime\sysproxy.journal.json`（`cleanExit` 标记）？启动守卫是否按
      「当前 ProxyServer 是否仍指向我们」分别处理（恢复 / 只清 journal 不动注册表）？
- [ ] 14. `sysproxy_set` 是否返回 `SysProxyState` 并同时更新 `RuntimeState.sysproxy` + 发 `sysproxy:state`（C.6/C.7）？
      **失败路径是否也回读真值并 emit**（不能让 UI 停在错的状态）？
- [ ] 15. `managed_by_us` 是否按「journal 存在且未 cleanExit 且 server 指向我们的回环端口」判定，
      而不是简单等价于 `enabled`（C.5 语义）？
- [ ] 16. `core:install-progress` 是否 ≤2 条/秒、phase 四值齐全、`done` 必发一次（C.7）？
- [ ] 17. `kernel/api.rs` 是否提供 `SelectorApi` 三方法 + `ApiError`，且签名与 C.6.1 冻结面一致？
      trait 是否为 **boxed-future** 形式（`ApiFuture<'_,T>`）、**没有**裸 `async fn`、**没有** `async-trait` 依赖？（v1.4）
- [ ] 18. `ApiError → AppError` 是否按 08.17.1 映射（Unauthorized→`E_CORE_API_UNAUTH`、
      Timeout→`E_NET_TIMEOUT`、其余→`E_CORE_START_FAILED` 带 detail）？有无自造新错误码？
- [ ] 18b. `proxies_selectable` 是否只保留 API `type == "Selector"` 且 `!hidden` 的组，
      且用**大小写敏感精确匹配**（不是 `to_lowercase() == "select"`，那是 YAML 值）？（C.6.1 `kind` 陷阱）
- [ ] 19. 版本 GC 是否保留「当前 + 上一版」？清理是否限定在 `core\` 与 `download\` 子目录内，
      **没有**对整个 `runtime\` 做 `remove_dir_all`（会误删 `switch-history.json` / journal）？
- [ ] 20. 单实例是否为 `Local\ClashZhengl.SingleInstance.v1` 命名互斥 + 唤起已有窗口？
- [ ] 21. `runtime\`、`logs\` 全文搜索是否无 secret 明文（08.15 第 8 条）？
- [ ] 22. 所有异步任务是否有取消路径或超时（C.13）？有无裸 `tokio::spawn` 无人回收？
- [ ] 23. 本章是否**没有**修改 `01-master-prompt.md`、`appendices/C-conventions.md` 或其它章节？
      发现契约缺口是否以 `CONTRACT-CHANGE-REQUEST:` 提出而非直接改？
