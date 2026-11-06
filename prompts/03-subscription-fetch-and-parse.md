# 03 · 订阅拉取、YAML 解析与流量/到期提取

> **本章用途**：定义「从一条订阅 URL 到本地 profile + UserInfo」的完整链路：
> HTTP 拉取规则（UA / 超时 / 代理 / 条件请求 / 上限 / 重试）、YAML 合法性与节点统计、
> `subscription-userinfo` 头解析与网页兜底、错误码映射、脱敏要求，以及可直接落地的 Rust 实现。
>
> **投喂时机**：P2 阶段开工前投喂（与 `04-multi-subscription-merge.md` 同一批）。
>
> **前置依赖**：必须已读 `01-master-prompt.md` + `appendices/C-conventions.md`
> （尤其 C.3 路径、C.5 `Subscription`/`UserInfo`、C.8 错误码、C.9 日志脱敏、C.10 单位约定）
> 与 `02-tech-stack-and-architecture.md`（§5 依赖白名单、§4 线程模型、§7 启动时序）。
> 本章不得修改契约；发现缺口输出 `CONTRACT-CHANGE-REQUEST:`。

---

## 1. 链路总览

```text
subscription_refresh(uid)
  │
  ├─ 1. store 解密 url_cipher（DPAPI，仅在此瞬间解密；用完即 drop）
  ├─ 2. meta.json 读取 ETag / Last-Modified / Content-Length
  ├─ 3. util::net::build_fetch_client(settings)  ← 共享 Client（连接复用）
  ├─ 4. fetch::fetch_once()  带条件请求 + 8 MB 上限 + 超时
  │      ├─ 304 Not Modified → 复用本地 yaml，只更新 last_update_at（成功）
  │      ├─ 2xx → 字节流累计校验大小 → 落盘前先在内存解析
  │      └─ 非 2xx / 网络错误 → 退避重试（最多 3 次：1s/2s/4s + 抖动）
  ├─ 5. parse::validate_and_parse()  → ParsedProfile{ proxies, groups, rules_present }
  │      ├─ 非法 / 缺 proxies / 节点数 0 → E_PARSE_YAML / E_PARSE_EMPTY
  │      └─ 通过 → 原子写 profiles/<uid>.yaml（原样保存，不重写）
  ├─ 6. userinfo::parse_userinfo_header(headers) → Option<UserInfo>
  │      └─ 缺失 → 网页兜底（默认关闭，需用户显式开启）→ 仍失败 E_USERINFO_MISSING
  ├─ 7. 原子写 profiles/<uid>.meta.json（UserInfo + ETag/Last-Modified/Content-Length + 校验信息）
  ├─ 8. 更新 index.json 中的 Subscription{node_count, group_names, last_update_status, mgmt}
  └─ 9. emit "subscription:updated"（成功失败都发；失败时 Subscription.last_error 带错误码）
```

> **顺序铁律**：**先解析成功，再落盘**。绝不允许「先覆盖 `profiles/<uid>.yaml` 再解析」——
> 那会在机场下发坏配置时把用户唯一可用的本地副本一起毁掉。

---

## 2. HTTP 拉取实现要点

### 2.1 参数表（全部来自 `AppSettings`，禁止硬编码）

| 参数 | 来源字段 | 默认值 | 约束 |
| --- | --- | --- | --- |
| User-Agent | `sub_user_agent` | `clash.meta/1.19.0` | 部分机场按 UA 下发不同格式；**不得**伪装成浏览器以外的随机 UA |
| 连接超时 | 固定 10 s | 10 s | 不随 `sub_fetch_timeout_ms` 变化 |
| 单次总超时 | `sub_fetch_timeout_ms` | 15 000 ms | 范围 3 000–60 000；超时映射 `E_NET_TIMEOUT` |
| 拉取代理 | `sub_fetch_proxy` | `None` | 形如 `http://127.0.0.1:1886`；`None` 时**不读环境变量**（见 §2.4） |
| 并发 | `sub_fetch_concurrency` | 2 | 上限 4 |
| 响应体上限 | 固定 8 MiB | 8 MiB | 超限 `E_HTTP_STATUS` 变体说明见 §2.6 |
| 重试次数 | 固定 3 | 1 次首发 + 3 次重试 | 退避 1s / 2s / 4s，各加 `0..300ms` 抖动 |
| 自动解压 | gzip | 开 | deflate 按需；**不**开 brotli/zstd（体积） |
| 重定向 | 最多 10 跳 | 10 | 机场常见 301 到 CDN |
| 连接复用 | 单 Client 复用 | `pool_max_idle_per_host = 4`，`pool_idle_timeout = 90s` | 禁止每个订阅 new 一个 Client |
| 条件请求 | ETag / Last-Modified | 有则带 | 服务器不支持则忽略 |

### 2.2 为什么必须复用 Client

每新建一个 `reqwest::Client` 都会新建连接池与 TLS 会话缓存。5 个订阅串行刷新时：
- 不复用：5 次 TCP + 5 次 TLS 握手（每次 100–300 ms，跨境更久）；
- 复用：同一 host 的后续请求走 keep-alive，省掉握手。

所以 `reqwest::Client` **只能**在 `util/net.rs` 构造一次，之后 `Arc<Client>` 传递。
同时必须注意：**Client 内绑定了代理配置**。若用户改了 `sub_fetch_proxy`，必须重建 Client（不是改请求）。

### 2.3 客户端构造（可直接落地）

```rust
// src-tauri/src/util/net.rs
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, CACHE_CONTROL};
use reqwest::redirect;
use reqwest::{Client, Proxy};

use crate::domain::errors::AppError;
use crate::domain::model::AppSettings;

/// 默认 UA：mihomo 生态通用值，部分机场按此下发 Clash 格式而非 base64 通用格式。
pub const DEFAULT_UA: &str = "clash.meta/1.19.0";
/// 响应体上限：8 MiB。超过即判定为异常响应（多为 HTML 错误页或误配的 base64 订阅）。
pub const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

/// 唯一允许创建联网 Client 的地方。每次 settings 变更后重建（见 app.rs）。
pub fn build_fetch_client(settings: &AppSettings) -> Result<Client, AppError> {
    let ua = if settings.sub_user_agent.trim().is_empty() {
        DEFAULT_UA
    } else {
        settings.sub_user_agent.as_str()
    };

    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    // 显式声明，避免部分 CDN 返回无 Content-Encoding 的已压缩体
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip, deflate"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));

    let mut builder = Client::builder()
        .user_agent(ua)
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_millis(settings.sub_fetch_timeout_ms as u64))
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .redirect(redirect::Policy::limited(10))
        .gzip(true)
        .deflate(true)
        // 订阅内容大小未知，禁用「先读 content-length 再决定」的隐式行为，
        // 由我们自己的字节流累计来强制 8 MiB 上限。
        .no_proxy(); // 先清空系统/环境代理，避免与用户显式设置冲突

    // 只有用户显式配置才走代理；None 表示直连（不读 HTTP_PROXY/HTTPS_PROXY）。
    if let Some(p) = settings.sub_fetch_proxy.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let proxy = Proxy::all(p).map_err(|e| {
            AppError::new(
                "E_NET_PROXY",
                "拉取代理不可用",
                Some(format!("invalid proxy syntax: {e}")),
                false,
            )
        })?;
        builder = builder.proxy(proxy);
    }

    builder.build().map_err(|e| {
        AppError::new(
            "E_UNSUPPORTED",
            "无法初始化网络组件",
            Some(e.to_string()),
            false,
        )
    })
}
```

> `reqwest 0.13` 起 `rustls` 为默认 TLS 后端（不再需要 `rustls-tls` 特性名，改用 `rustls`），
> 且默认使用 `rustls-platform-verifier` 校验证书链——Windows 上等于沿用系统根证书，符合企业内网场景。

### 2.4 代理语义（必须写死在代码注释里）

| 场景 | 行为 |
| --- | --- |
| `sub_fetch_proxy = None` | **直连**，忽略 `HTTP_PROXY`/`HTTPS_PROXY`/系统代理（`no_proxy()` 之后不再添加 proxy） |
| `sub_fetch_proxy = Some("http://127.0.0.1:1886")` | 全部订阅请求走该 HTTP 代理 |
| 代理不可用 | 连接失败 → `E_NET_PROXY`（**不要**自动回退直连：用户配代理就是为了绕过被墙的机场域名） |
| 内网/本地地址 | 若订阅 host 是 `localhost`/`127.0.0.1`/`*.lan`，仍走配置的代理（保持行为可预测，不做魔法例外） |

> 为什么不读环境变量：便携版用户机器上常有残留的 `HTTP_PROXY`，静默生效会造成「换台电脑就失败」的幽灵 bug。
> 显式配置 > 隐式环境，这是可解释性要求（01 铁律 7）。

### 2.5 条件请求（ETag / If-Modified-Since）

```rust
// src-tauri/src/subscription/fetch.rs（节选）
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::StatusCode;

/// meta.json 中持久化的校验信息（C.3 的 <uid>.meta.json 一部分）
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HttpCacheMeta {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_length: Option<u64>,
}

pub fn apply_conditional(req: reqwest::RequestBuilder, cache: &HttpCacheMeta) -> reqwest::RequestBuilder {
    let mut req = req;
    if let Some(etag) = cache.etag.as_deref() {
        // 服务器可能返回弱校验值 W/"..."，必须原样回传
        req = req.header(IF_NONE_MATCH, etag);
    }
    if let Some(lm) = cache.last_modified.as_deref() {
        // 只需字符串原样回传；不解析成时间，避免时区/格式坑
        req = req.header(IF_MODIFIED_SINCE, lm);
    }
    req
}

pub fn extract_cache_meta(headers: &reqwest::header::HeaderMap) -> HttpCacheMeta {
    HttpCacheMeta {
        etag: headers
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string()),
        last_modified: headers
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string()),
        content_length: headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok()),
    }
}
```

**304 的处理规则**：

| 情况 | 处理 |
| --- | --- |
| 304 且本地 `profiles/<uid>.yaml` 存在且非空 | 成功；不重写 yaml；`last_update_at = now`；`last_update_status = Ok`；meta 里的 ETag 保持不变 |
| 304 但本地 yaml 缺失/为空（用户手工删了） | **忽略 304 重发无条件请求**（否则该订阅永久无法恢复） |
| 服务器返回 304 却带 body | 忽略 body（RFC 允许但实务罕见） |

### 2.6 响应体读取与 8 MiB 上限

```rust
// src-tauri/src/subscription/fetch.rs（节选）
use crate::domain::errors::AppError;
use crate::util::net::MAX_BODY_BYTES;

/// 流式读取响应体，边读边累计，超限立刻中止（不把超大 body 全读进内存）。
/// 注意：本函数**不需要** `futures` / `bytes` crate —— 只用 `Response::chunk()` 即可，
/// 这也是不把 `futures` 加入依赖白名单的原因。
pub async fn read_body_limited(mut resp: reqwest::Response) -> Result<Vec<u8>, AppError> {
    // 先做一次廉价预判：Content-Length 超限直接失败，省流量
    if let Some(len) = resp.content_length() {
        if len > MAX_BODY_BYTES {
            return Err(AppError::new(
                "E_HTTP_STATUS",
                format!("订阅文件过大（{} 字节，上限 {} 字节）", len, MAX_BODY_BYTES),
                Some("body exceeds MAX_BODY_BYTES".into()),
                false,
            ));
        }
    }

    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    // 用 chunk() 而非 bytes()：bytes() 会先聚合完整 body，超大响应会直接吃掉内存
    while let Some(chunk) = resp.chunk().await.map_err(map_reqwest_err)? {
        if buf.len() as u64 + chunk.len() as u64 > MAX_BODY_BYTES {
            return Err(AppError::new(
                "E_HTTP_STATUS",
                format!("订阅文件超过 {} 字节上限", MAX_BODY_BYTES),
                Some("body exceeds MAX_BODY_BYTES while streaming".into()),
                false,
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// 把 reqwest::Error 映射到 C.8 的错误码。顺序很重要：
/// is_timeout > is_dns > is_connect(代理/TLS) > is_decode > 兜底。
pub fn map_reqwest_err(e: reqwest::Error) -> AppError {
    let detail = Some(redact_error_text(&e.to_string()));
    if e.is_timeout() {
        return AppError::new("E_NET_TIMEOUT", "连接订阅服务器超时", detail, true);
    }
    if e.is_dns() {
        return AppError::new("E_NET_DNS", "无法解析订阅域名，请检查网络或 DNS", detail, true);
    }
    if e.is_connect() {
        // 有代理配置时，连接失败 99% 是代理问题；无代理时更可能是 TLS/证书
        let msg = e.to_string().to_ascii_lowercase();
        if msg.contains("tls") || msg.contains("certificate") || msg.contains("handshake") {
            return AppError::new("E_NET_TLS", "订阅服务器 TLS 校验失败", detail, false);
        }
        return AppError::new("E_NET_PROXY", "拉取代理不可用", detail, true);
    }
    if e.is_decode() {
        return AppError::new("E_NET_TIMEOUT", "订阅内容解码失败（可能是压缩格式异常）", detail, true);
    }
    AppError::new("E_NET_TIMEOUT", "拉取订阅失败", detail, true)
}

/// 错误文本也要脱敏：reqwest 的 URL 类错误可能带完整 URL（含 token）。
fn redact_error_text(s: &str) -> String {
    crate::util::log::redact::scrub(s)
}
```

> **不要**用 `resp.bytes().await`：它在返回前会聚合成一个 `Bytes`，
> 若服务器不返回 `Content-Length`（chunked）且下发 500 MB HTML，内存会瞬间被打爆。

### 2.7 重试与指数退避

| 尝试 | 延迟（前） | 说明 |
| --- | --- | --- |
| 第 1 次 | 0 | 立即 |
| 第 2 次 | `1000 ms + rand(0..300) ms` | |
| 第 3 次 | `2000 ms + rand(0..300) ms` | |
| 第 4 次 | `4000 ms + rand(0..300) ms` | 之后放弃 |

**可重试判定**（只有这些才重试）：

| 类别 | 是否重试 | 理由 |
| --- | --- | --- |
| `E_NET_TIMEOUT` | ✅ | 网络抖动 |
| `E_NET_DNS` | ✅ | 本地 DNS 瞬时失败 |
| `E_NET_PROXY` | ✅ | 代理进程可能刚启动 |
| HTTP 5xx | ✅ | 机场服务端临时错误 |
| HTTP 429 | ✅（延迟翻倍） | 限流；额外 `Retry-After` 优先 |
| HTTP 408 | ✅ | 请求超时 |
| `E_NET_TLS` | ❌ | 证书问题重试无用，重试只会浪费时间与日志 |
| HTTP 4xx（除 408/429） | ❌ | 401/403/404 是 token 或订阅已失效，需要用户介入 |
| `E_PARSE_*` | ❌ | 内容问题，重试拿到同样内容 |
| 超出 8 MiB | ❌ | 重试仍超限 |

```rust
// src-tauri/src/subscription/fetch.rs（节选）
use std::time::Duration;

/// 指数退避 + 抖动。纯函数，可单测（C.10 要求所有抖动随机化）。
pub fn backoff_delay(attempt: u32, jitter_ms: u64) -> Duration {
    // attempt: 0 => 1s, 1 => 2s, 2 => 4s
    let base_ms = 1000u64.saturating_mul(1u64 << attempt.min(2));
    Duration::from_millis(base_ms + jitter_ms)
}

pub fn is_retryable(err: &AppError, status: Option<u16>) -> bool {
    if let Some(code) = status {
        return code >= 500 || code == 408 || code == 429;
    }
    matches!(
        err.code.as_str(),
        "E_NET_TIMEOUT" | "E_NET_DNS" | "E_NET_PROXY" | "E_BUSY"
    )
}

/// 3 次重试的统一封装。取消安全：每段 sleep 都包在 select! 里（见 05 章 CancellationToken）。
pub async fn with_retry<F, Fut, T>(
    cancel: &tokio_util::sync::CancellationToken,
    mut op: F,
) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, AppError>>,
{
    let mut last_err: Option<AppError> = None;
    for attempt in 0u32..=3 {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let retryable = is_retryable(&e, None);
                tracing::warn!(
                    code = %e.code,
                    retry = format!("{}/3", attempt),
                    retryable,
                    "fetch attempt failed"
                );
                if !retryable || attempt == 3 {
                    return Err(e);
                }
                last_err = Some(e);
                // 抖动 0..300ms：C.10 要求周期性与重试都加随机偏移
                let jitter = u64::from(getrandom::u32().unwrap_or(0) % 300);
                let delay = backoff_delay(attempt, jitter);
                tokio::select! {
                    _ = cancel.cancelled() => {
                        return Err(last_err.unwrap_or_else(|| AppError::new("E_BUSY", "操作已取消", None, true)));
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| AppError::new("E_NET_TIMEOUT", "拉取订阅失败", None, true)))
}
```

> `getrandom` 0.4 提供 `fill(&mut [u8])` 与 `u32()/u64()`；若版本无 `u32()`，用
> `let mut b=[0u8;4]; getrandom::fill(&mut b)?; u32::from_le_bytes(b) % 300`。

---

## 3. YAML 解析与合法性校验

### 3.1 校验分级

| 级别 | 检查项 | 不通过的错误码 | 处置 |
| --- | --- | --- | --- |
| L0 硬性 | 响应字节可被 `serde_yaml_ng::from_slice::<Value>` 解析 | `E_PARSE_YAML` | 失败，**不落盘**，保留旧 profile |
| L0 硬性 | 顶层是 Mapping | `E_PARSE_YAML` | 同上 |
| L0 硬性 | 存在 `proxies` 且是 Sequence | `E_PARSE_YAML` | 同上 |
| L0 硬性 | `proxies.len() > 0` | `E_PARSE_EMPTY` | 同上 |
| L1 建议 | 每个 proxy 有 `name`(非空字符串) 与 `type`(字符串) | 计数告警 | 记录 `skipped=N`，**不失败** |
| L1 建议 | `proxy-groups` 若存在必须是 Sequence | 计数告警 | 忽略该段，记录 WARN |
| L1 建议 | `rules` 若存在必须是 Sequence | 计数告警 | 忽略该段，记录 WARN |
| L2 提示 | 节点名重复 | 无 | 交给 04 章做去重与冲突前缀 |
| L2 提示 | 顶层出现 `port`/`socks-port` 等**我们会被覆盖**的字段 | 无 | 记录 Info：合并时会被忽略（04 章） |

> **不要**对 L1/L2 判失败。机场配置千奇百怪，过度严格会导致「用户明明能用其它客户端」的投诉。

### 3.2 数据结构与解析实现

```rust
// src-tauri/src/subscription/parse.rs
use serde_yaml_ng::{Mapping, Value};

use crate::domain::errors::AppError;
use crate::domain::model::ProxyKind;

/// 解析结果：只抽出我们自己要用的部分，其余原样保留在 raw 里（04 章合并时按需读取）。
#[derive(Debug, Clone)]
pub struct ParsedProfile {
    /// 有效节点数（name 与 type 都合法的条目数）
    pub node_count: u32,
    /// 各类型统计，按 ProxyKind 计数（供 UI 展示 "SS 42 · VMess 30"）
    pub kinds: Vec<(ProxyKind, u32)>,
    /// 节点名（顺序保持文件顺序），04 章去重与命名冲突都基于它
    pub node_names: Vec<String>,
    /// 策略组名（顺序保持），04 章用于判断是否需要补充组
    pub group_names: Vec<String>,
    pub has_rules: bool,
}

fn as_mapping(v: &Value) -> Option<&Mapping> {
    v.as_mapping()
}

fn as_seq<'a>(v: &'a Value) -> Option<&'a Vec<Value>> {
    v.as_sequence()
}

/// 把机场的 type 字符串映射到契约枚举（C.5 的 ProxyKind）。
/// 未知类型一律 Other，**不失败**（mihomo 支持的类型远多于我们枚举的）。
pub fn map_kind(raw: &str) -> ProxyKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "ss" | "shadowsocks" => ProxyKind::Ss,
        "vmess" => ProxyKind::Vmess,
        "vless" => ProxyKind::Vless,
        "trojan" => ProxyKind::Trojan,
        "hysteria2" | "hy2" => ProxyKind::Hysteria2,
        "tuic" => ProxyKind::Tuic,
        "wireguard" => ProxyKind::Wireguard,
        "http" => ProxyKind::Http,
        "socks5" | "socks" => ProxyKind::Socks5,
        _ => ProxyKind::Other,
    }
}

/// 核心校验 + 统计。纯函数：输入字节，输出结果或错误；不碰文件系统、不读时间。
pub fn validate_and_parse(bytes: &[u8]) -> Result<ParsedProfile, AppError> {
    // 常见误配 1：机场下发了 HTML 错误页（CDN 502 / 维护页）
    let head = &bytes[..bytes.len().min(512)];
    if head.starts_with(b"<!DOCTYPE") || head.starts_with(b"<html") || head.starts_with(b"{") {
        // 注意：以 "{" 开头说明是 JSON（可能是 base64/通用订阅或错误对象），不是 Clash YAML
        return Err(AppError::new(
            "E_PARSE_YAML",
            "订阅内容不是有效的 Clash 配置",
            Some(format!(
                "unexpected leading bytes: {:?}",
                String::from_utf8_lossy(&head[..head.len().min(32)])
            )),
            false,
        ));
    }

    let doc: Value = serde_yaml_ng::from_slice(bytes).map_err(|e| {
        AppError::new(
            "E_PARSE_YAML",
            "订阅内容不是有效的 Clash 配置",
            Some(format!("yaml: {e}")), // serde_yaml_ng 的错误已含行列号，且不含 token
            false,
        )
    })?;

    let root = as_mapping(&doc).ok_or_else(|| {
        AppError::new(
            "E_PARSE_YAML",
            "订阅内容不是有效的 Clash 配置",
            Some("root is not a mapping".into()),
            false,
        )
    })?;

    let proxies_val = root.get(Value::String("proxies".into())).ok_or_else(|| {
        AppError::new(
            "E_PARSE_YAML",
            "订阅内容不是有效的 Clash 配置",
            Some("missing `proxies`".into()),
            false,
        )
    })?;

    let proxies = as_seq(proxies_val).ok_or_else(|| {
        AppError::new(
            "E_PARSE_YAML",
            "订阅内容不是有效的 Clash 配置",
            Some("`proxies` is not a sequence".into()),
            false,
        )
    })?;

    let mut names: Vec<String> = Vec::with_capacity(proxies.len());
    let mut counts: std::collections::BTreeMap<&'static str, (ProxyKind, u32)> =
        std::collections::BTreeMap::new();
    let mut skipped = 0u32;

    for (idx, item) in proxies.iter().enumerate() {
        let Some(m) = as_mapping(item) else {
            skipped += 1;
            tracing::debug!(idx, "skipping non-mapping proxy entry");
            continue;
        };
        let name = m
            .get(Value::String("name".into()))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let kind_raw = m.get(Value::String("type".into())).and_then(Value::as_str);

        let (Some(name), Some(kind_raw)) = (name, kind_raw) else {
            skipped += 1;
            tracing::debug!(idx, "skipping proxy without name/type");
            continue;
        };
        // 契约要求 ProxyResult.node_key 唯一可读；name 里带换行会破坏日志单行结构（C.9）
        if name.contains('\n') || name.contains('\r') {
            skipped += 1;
            continue;
        }

        names.push(name.to_string());
        let kind = map_kind(kind_raw);
        counts.entry(kind_key(kind)).or_insert((kind, 0)).1 += 1;
    }

    if names.is_empty() {
        return Err(AppError::new(
            "E_PARSE_EMPTY",
            "订阅中没有可用节点，请检查订阅是否为空或已过期",
            Some(format!("proxies={}, skipped={skipped}", proxies.len())),
            false,
        ));
    }
    if skipped > 0 {
        tracing::warn!(skipped, total = proxies.len(), "some proxy entries were skipped");
    }

    let group_names = root
        .get(Value::String("proxy-groups".into()))
        .and_then(as_seq)
        .map(|seq| {
            seq.iter()
                .filter_map(|g| g.as_mapping())
                .filter_map(|m| m.get(Value::String("name".into())).and_then(Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let has_rules = root
        .get(Value::String("rules".into()))
        .and_then(as_seq)
        .is_some_and(|s| !s.is_empty());

    let kinds: Vec<(ProxyKind, u32)> = counts.into_values().collect();

    Ok(ParsedProfile {
        node_count: names.len() as u32,
        kinds,
        node_names: names,
        group_names,
        has_rules,
    })
}

fn kind_key(k: ProxyKind) -> &'static str {
    match k {
        ProxyKind::Ss => "ss",
        ProxyKind::Vmess => "vmess",
        ProxyKind::Vless => "vless",
        ProxyKind::Trojan => "trojan",
        ProxyKind::Hysteria2 => "hysteria2",
        ProxyKind::Tuic => "tuic",
        ProxyKind::Wireguard => "wireguard",
        ProxyKind::Http => "http",
        ProxyKind::Socks5 => "socks5",
        ProxyKind::Other => "other",
    }
}
```

### 3.3 类型统计示例（UI 展示口径）

| 场景 | 展示文案（09 章） |
| --- | --- |
| 混合 | `VMess 30 · SS 12 · Trojan 8` |
| 单一 | `Trojan 40` |
| 含未知 | `SS 12 · 其它 3`（`Other` 归类为「其它」） |
| 空 | 不出现该行（节点数 0 会先走 `E_PARSE_EMPTY`） |

> 统计字段落库时**不要**塞进 `Subscription` 结构（契约 C.5 没有该字段）。
> 需要展示时由 `subscription_list` 现场读 meta 或由 manager 缓存的内存投影提供。
> **禁止**为展示需求擅自给 `Subscription` 加字段——那属于契约变更。

---

## 4. `subscription-userinfo` 头解析

### 4.1 头格式规范

```http
subscription-userinfo: upload=1234; download=5678; total=1000000000000; expire=1735689600
```

| 字段 | 类型 | 语义 | 缺省 | 单位 |
| --- | --- | --- | --- | --- |
| `upload` | u64 | 已上传字节 | 缺失 → `None` | 字节 |
| `download` | u64 | 已下载字节 | 缺失 → `None` | 字节 |
| `total` | u64 | 总配额字节；**`0` 表示不限量** | 缺失 → `None` | 字节 |
| `expire` | i64 | 到期 **Unix 秒**（UTC）；`0`/缺失表示无限期 | 缺失 → `None` | 秒 |

**字段名不区分大小写**（实测有 `Upload=`、`TOTAL=`）；
**分隔符是 `;`**，允许结尾多余分号、允许键值间空格、允许值为空（`total=`）。

参考实现（Clash Verge Rev `src-tauri/src/config/prfitem.rs`）的做法：
遍历响应头，对 key 做 `to_ascii_lowercase()`，再用 `strip_suffix("subscription-userinfo")`
判断（允许 `x-amz-meta-` 等前缀），命中后取第一个，然后按字段名逐个 `parse_str`，失败填 0。

> **我们与 Verge 的差异（有意为之）**：
> Verge 用 `u64` + 失败填 0，丢失了「字段缺失」与「值为 0」的区别；
> 契约 C.5 的 `UserInfo` 全是 `Option<i64>`，因此我们**保留 `None`**，
> 以便 UI 区分「机场没给这项」与「这项真的是 0」。

### 4.2 解析函数（纯函数，可单测）

```rust
// src-tauri/src/subscription/userinfo.rs
use crate::domain::model::{UserInfo, UserInfoSource};

/// 解析 `subscription-userinfo` 原始字符串。
/// 纯函数：不读时间、不读 header 容器、不做 IO，便于 100% 单测。
/// 返回值中 `fetched_at` 由调用方填充（本函数填 0，调用方必须覆盖）。
pub fn parse_userinfo_header(raw: &str) -> Option<UserInfo> {
    let mut upload: Option<i64> = None;
    let mut download: Option<i64> = None;
    let mut total: Option<i64> = None;
    let mut expire_at: Option<i64> = None;
    let mut seen_any = false;

    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue; // 容错：开头/结尾/连续分号
        }
        let Some((k, v)) = part.split_once('=') else {
            continue; // 容错：无等号的垃圾片段
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim();
        if val.is_empty() {
            continue; // 容错：`total=` 视为缺失而非 0
        }
        seen_any = true;
        match key.as_str() {
            "upload" => upload = parse_i64_lossy(val),
            "download" => download = parse_i64_lossy(val),
            "total" => total = parse_i64_lossy(val),
            "expire" => expire_at = normalize_expire(parse_i64_lossy(val)),
            _ => {} // 未知键忽略（机场常塞 `plan=`、`name=` 等私有字段）
        }
    }

    if !seen_any {
        return None;
    }

    Some(UserInfo {
        upload,
        download,
        total,
        expire_at,
        web_page_url: None,
        fetched_at: 0, // 由调用方覆盖
        source: UserInfoSource::Header,
        raw: Some(raw.to_string()),
    })
}

/// 数值解析：只接受十进制数字（可带前导 `+`），其余一律视为缺失。
/// 不用 `unwrap()`，不用 `?` 直接丢弃（我们要区分「非数字」与「0」）。
fn parse_i64_lossy(s: &str) -> Option<i64> {
    let t = s.trim().trim_start_matches('+');
    if t.is_empty() {
        return None;
    }
    // 拒绝 "12.5"、"1e9"、"1,000"（机场极少这么干，但错解析成 12 更糟）
    if !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<i64>().ok().filter(|n| *n >= 0)
}

/// expire 单位纠正：
/// - 秒级：~1.7e9（2023–2033 年区间）
/// - 毫秒级：~1.7e12（误用毫秒的机场真实存在）
/// 判定阈值：> 100_000_000_000（1e11）视为毫秒，除以 1000。
fn normalize_expire(v: Option<i64>) -> Option<i64> {
    match v {
        None | Some(0) => None, // 0 表示无限期
        Some(n) if n > 100_000_000_000 => Some(n / 1000),
        Some(n) => Some(n),
    }
}

/// 从响应头容器中取出 UserInfo（处理 header 名前后缀差异）。
pub fn userinfo_from_headers(headers: &reqwest::header::HeaderMap, now: i64) -> Option<UserInfo> {
    for (k, v) in headers.iter() {
        let key = k.as_str().to_ascii_lowercase();
        // 允许 `subscription-userinfo` 与 `x-*-subscription-userinfo`（对象存储元数据头）
        let matched = key == "subscription-userinfo"
            || key
                .strip_suffix("subscription-userinfo")
                .is_some_and(|prefix| prefix.ends_with('-'));
        if !matched {
            continue;
        }
        let Ok(raw) = v.to_str() else { continue };
        if let Some(mut info) = parse_userinfo_header(raw) {
            info.fetched_at = now;
            return Some(info);
        }
    }
    None
}
```

### 4.3 单测用例表（必须逐条实现）

| # | 输入（`raw` 字符串） | 期望 | 说明 |
| --- | --- | --- | --- |
| 1 | `upload=1234; download=5678; total=1000000; expire=1735689600` | 四项全有，`expire_at=Some(1735689600)` | 正常样本 |
| 2 | `upload=1; download=2` | `total=None, expire_at=None` | 缺字段 → `None`（不是 0） |
| 3 | `upload=; download=; total=; expire=` | 全 `None`，但 `Some(UserInfo)` | 空值；`seen_any=false`? 注意：空值不计入 `seen_any`，因此应返回 `None` |
| 4 | `total=0` | `total=Some(0)`（UI 显示「不限量」） | `0` ≠ 缺失 |
| 5 | `expire=1735689600000` | `expire_at=Some(1735689600)` | 毫秒误用纠正 |
| 6 | `; upload=1;; download=2; total=3; expire=4;;` | 正常解析四项 | 多余/连续分号 |
| 7 | `UPLOAD=10; Download=20; TOTAL=30; EXPIRE=40` | 正常解析（键名大小写不敏感） | 大小写 |
| 8 | `upload=1.5; total=abc` | `upload=None, total=None` | 非数字不猜测 |
| 9 | `upload=1e9; total=1,000` | 均 `None` | 科学计数法/千分位不猜 |
| 10 | `plan=pro; upload=1; download=2; total=3; expire=4` | 四项正常，未知键忽略 | 私有键 |
| 11 | `"  upload = 1 ; total = 2 "` | 正常（空格容错） | 空白 |
| 12 | `expire=0` | `expire_at=None`（无限期） | 0 语义 |
| 13 | `upload=-5` | `upload=None` | 负数拒绝 |
| 14 | 空字符串 `""` | `None` | 无值 |
| 15 | `total=9999999999999999999999` | `total=None`（溢出 i64） | 溢出 |

> 用例 3 的判定细节：所有字段值为空 ⇒ `seen_any == false` ⇒ 返回 `None`，
> 保证「头存在但没内容」与「头不存在」语义一致，UI 统一走「未提供流量信息」。
> 若希望区分，改用 `Some(UserInfo{ all None })` + `source=Header`；**本项目选择返回 `None`**（更简单，少一个 UI 分支）。

### 4.4 `web_page_url` 的来源

部分机场在响应头额外下发 `profile-web-page-url: https://xxx/user`。处理规则：

| 情况 | 行为 |
| --- | --- |
| 有 `profile-web-page-url` 且是 `http(s)://` | 写入 `UserInfo.web_page_url`；**仅用于「打开机场面板」按钮，绝不自动访问** |
| 无 | `web_page_url = None` |
| 非法（`javascript:`/`file:`） | 丢弃并 WARN |

> 这是安全边界：外部 URL 不得直接 `shell::open`，必须先校验 scheme（11 章有落地实现）。

### 4.5 网页兜底（MVP **不实现**，仅保留设计）

> **状态：`TODO(待确认)` —— MVP 不提供该功能（v1.4 Lead 裁决）。**
> 契约没有 `sub_userinfo_web_fallback` 字段，且**不进契约**。本节保留完整设计仅为将来可能的
> P6 迭代提供依据；**当前实现必须按「永不触发」处理**：头缺失即 `E_USERINFO_MISSING` 静默降级，
> 不做任何网页抓取。若将来启用，需先走 `CONTRACT-CHANGE-REQUEST:` 新增
> `AppSettings.sub_userinfo_web_fallback: bool`（默认 `false`）并升 `schema_version`。

| 项 | 规定（若将来启用） |
| --- | --- |
| 触发条件 | 头缺失 **且** 用户在该订阅上显式开启「尝试从网页获取流量信息」 |
| 实现 | GET 订阅 URL 的 `profile-web-page-url`，解析 HTML 中的「剩余流量 / 到期时间」文本 |
| 解析方式 | 手写关键词扫描（`剩余流量`、`到期时间`、`到期`、`流量`、`GB`、`TB`），**不引入 HTML 解析库** |
| 成功率预期 | **低**（10–25%）。必须视作 best-effort，不得报错打扰用户 |
| 失败 | `source` 保持 `Unknown`，`mgmt=None`，**不**写 `E_USERINFO_MISSING` 到订阅（那是静默降级，不是错误） |
| 硬性限制 | 只抓 1 次、只抓 ≤256 KiB、只抓同源 URL；解析结果**必须**标注 `source=WebPage`，UI 上标注「来自网页，可能不准」 |
| 安全 | 结果中的数字不得参与任何计费/警告逻辑（只有 `Header` 来源可触发 `traffic_warn_percent` 通知） |

```rust
// src-tauri/src/subscription/userinfo.rs（兜底，节选）
/// 极简网页兜底：只认「数字 + 单位」的紧邻模式。
/// 明确不做：DOM 解析、JS 执行、登录态、翻页。
pub fn parse_userinfo_webpage(html: &str, now: i64) -> Option<UserInfo> {
    fn extract_gb(s: &str, keys: &[&str]) -> Option<i64> {
        for key in keys {
            let Some(pos) = s.find(key) else { continue };
            let tail = &s[pos + key.len()..s.len().min(pos + key.len() + 64)];
            let mut num = String::new();
            for ch in tail.chars() {
                if ch.is_ascii_digit() || ch == '.' {
                    num.push(ch);
                } else if !num.is_empty() {
                    break;
                }
            }
            let gb = num.parse::<f64>().ok()?;
            // 单位判断：抓数字后 8 个字符内出现 TB/GB/MB
            let unit_zone = &tail[..tail.len().min(24)];
            let bytes = if unit_zone.contains("TB") {
                gb * 1024.0 * 1024.0 * 1024.0 * 1024.0
            } else if unit_zone.contains("MB") {
                gb * 1024.0 * 1024.0
            } else {
                gb * 1024.0 * 1024.0 * 1024.0
            };
            return Some(bytes as i64);
        }
        None
    }

    let total = extract_gb(html, &["总流量", "总配额", "流量总额"]);
    let used = extract_gb(html, &["已用流量", "已使用", "已用"]);
    if total.is_none() && used.is_none() {
        return None;
    }
    Some(UserInfo {
        upload: None,
        download: used,
        total,
        expire_at: None, // 网页日期格式过于多样，兜底不猜；宁缺勿错
        web_page_url: None,
        fetched_at: now,
        source: UserInfoSource::WebPage,
        raw: None,
    })
}
```

> **真实样本字段差异**（供附录 B 互参）：部分机场把已用流量只放在 `download`（把上传算进去再给 download），
> 也有机场 `upload` 恒为 0。因此「已用 = upload + download」的派生规则（C.5）在缺失任一项时显示 `—`，
> 这条规则**不要**在 UI 层改成「缺失当 0」。

---

## 5. 错误码映射表

| 触发点 | code | message（中文，C.8 原文） | retryable | 是否写 `last_update_status` |
| --- | --- | --- | --- | --- |
| DNS 解析失败 | `E_NET_DNS` | 无法解析订阅域名，请检查网络或 DNS | true | Failed |
| 连接/读取超时 | `E_NET_TIMEOUT` | 连接订阅服务器超时 | true | Failed |
| TLS 握手/证书 | `E_NET_TLS` | 订阅服务器 TLS 校验失败 | false | Failed |
| 代理不可用 | `E_NET_PROXY` | 拉取代理（如 127.0.0.1:1886）不可用 | true | Failed |
| 非 2xx | `E_HTTP_STATUS` | 订阅服务器返回 HTTP {status} | 5xx/408/429 = true | Failed |
| 响应超 8 MiB | `E_HTTP_STATUS` | 订阅文件过大（…字节，上限 8388608 字节） | false | Failed |
| YAML 非法/缺 proxies | `E_PARSE_YAML` | 订阅内容不是有效的 Clash 配置 | false | Failed |
| 节点数 0 | `E_PARSE_EMPTY` | 订阅中没有可用节点，请检查订阅是否为空或已过期 | false | Failed |
| 头缺失且兜底失败 | `E_USERINFO_MISSING` | 该订阅未提供流量信息 | false | **保持 Ok**（不影响订阅可用性） |
| DPAPI 解密失败 | `E_STORE_DECRYPT` | 订阅地址无法解密，请重新填写 | false | Failed |
| 写盘失败 | `E_STORE_IO` | 配置写入失败，请检查磁盘权限 | true | Failed |
| 同一订阅正在刷新 | `E_BUSY` | 正在执行中，请稍候 | true | **不改状态** |

**落地约定**：
- `Subscription.last_error` 存 `"{code}: {message}"`（如 `E_NET_TIMEOUT: 连接订阅服务器超时`），
  前端按 `:` 前缀取 code 再映射本地文案（便于 i18n）。
- **禁止**把 `detail` 写进 `last_error`（可能含 URL）。`detail` 只进日志，且必须过脱敏器。

```rust
// src-tauri/src/domain/errors.rs（骨架）
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>, detail: Option<String>, retryable: bool) -> Self {
        Self { code: code.to_string(), message: message.into(), detail, retryable }
    }

    /// 生成可持久化到 Subscription.last_error 的短串（不含 detail）
    pub fn to_short(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}
```

---

## 6. 落盘与原子写

### 6.1 写盘顺序（关键：先数据，后索引）

```text
1) profiles/<uid>.yaml.tmp  写入原始响应字节 → fsync → rename 为 profiles/<uid>.yaml
2) profiles/<uid>.meta.json.tmp 写入 UserInfo + HttpCacheMeta → fsync → rename
3) profiles/index.json.tmp 写入更新后的 Vec<Subscription> → fsync → rename
```

| 中断点 | 后果 | 是否可恢复 |
| --- | --- | --- |
| 1 之前 | 无变化 | ✅ |
| 1 之后 2 之前 | yaml 新、meta 旧 | ✅ 下次刷新会重新拉取（ETag 可能已变，最多多拉一次） |
| 2 之后 3 之前 | yaml/meta 新、index 旧 | ✅ 启动时 index 里 `node_count` 过期，下次刷新修正；yaml 可用 |
| 3 之后 | 完成 | ✅ |

> 若顺序反过来（先写 index），可能出现「索引说有 120 个节点，但 yaml 是旧的 80 个」，UI 会闪数字。

### 6.2 原子写实现（04 章复用同一份）

```rust
// src-tauri/src/store/json_store.rs
use std::io::Write;
use std::path::Path;

use crate::domain::errors::AppError;

/// 原子写：`<path>.tmp` → flush → sync_all → rename。
/// Windows 上 `rename` 到已存在文件是原子的（MoveFileEx + REPLACE_EXISTING 语义）。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|s| s.to_str()).unwrap_or("bin")
    ));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::new("E_STORE_IO", "配置写入失败，请检查磁盘权限", Some(e.to_string()), true)
        })?;
    }

    let mut f = std::fs::File::create(&tmp).map_err(|e| {
        AppError::new("E_STORE_IO", "配置写入失败，请检查磁盘权限", Some(e.to_string()), true)
    })?;
    f.write_all(bytes).map_err(|e| {
        AppError::new("E_STORE_IO", "配置写入失败，请检查磁盘权限", Some(e.to_string()), true)
    })?;
    f.flush().map_err(|e| {
        AppError::new("E_STORE_IO", "配置写入失败，请检查磁盘权限", Some(e.to_string()), true)
    })?;
    // 必须 fsync，否则断电可能留下「文件名对、内容空」的文件
    f.sync_all().map_err(|e| {
        AppError::new("E_STORE_IO", "配置写入失败，请检查磁盘权限", Some(e.to_string()), true)
    })?;
    drop(f);

    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::new("E_STORE_IO", "配置写入失败，请检查磁盘权限", Some(e.to_string()), true)
    })
}
```

> 目标文件若被内核/其它进程占用，`rename` 在 Windows 上会失败（`ERROR_SHARING_VIOLATION`）。
> 处置：重试 3 次（50/150/400 ms），仍失败返回 `E_STORE_IO` 并在 UI 提示「请关闭占用该文件的程序」。

### 6.3 损坏恢复（C.3 要求）

| 文件 | 损坏表现 | 处置 |
| --- | --- | --- |
| `settings.json` | JSON 解析失败 | 改名 `settings.json.corrupt-<unix_ts>`，用默认 `AppSettings` 继续，`notify` 提示「设置已重置」 |
| `profiles/index.json` | 解析失败 | 改名备份，`Vec::new()`，UI 显示空列表 + 提示重新添加订阅 |
| `profiles/<uid>.yaml` | 非 YAML / 缺 proxies | **不删**（可能是机场临时故障），标记该订阅 `Failed`，保留文件供诊断导出 |
| `profiles/<uid>.meta.json` | 解析失败 | 视为 `HttpCacheMeta::default()` + `mgmt=None`，下次刷新重建 |

---

## 7. 安全与脱敏要求（硬性）

| 项 | 要求 | 违规后果 |
| --- | --- | --- |
| URL 落盘 | 只存 `url_cipher`（DPAPI）+ `url_hint`（脱敏） | 泄露 token = 账号被盗 |
| `url_hint` 生成 | 保留 `scheme://host[:port]/path` 的前 1 段，token 段替换为 `******`；query 全部丢弃 | 展示与排障需要 |
| 解密时机 | 仅在发起请求的瞬间解密，请求完成后 `zeroize` 或立即 drop | 长期驻留内存 |
| 请求日志 | 只记 `uid`、`host`、`status`、`elapsed_ms`、`bytes`，**绝不记完整 URL** | C.9 |
| 错误 detail | 必须过 `redact::scrub()`（正则替换 `token|access_token|key|secret|passwd|password|auth` 的值） | C.9 |
| panic 信息 | 不得包含 URL（panic hook 里再 scrub 一次） | 01 §8 |
| 诊断包 | 导出 zip 前对 `logs/`、`index.json` 做全文二次扫描 | 01 §7 质量门禁「人工自检第 3 条：脱敏检查」 |
| 代理配置 | `sub_fetch_proxy` 若含 `user:pass@`，日志与 UI 均需掩码 | C.9 |

```rust
// src-tauri/src/util/log/redact.rs
/// 脱敏：把疑似凭据的「值」替换为 `******`。
/// 只做保守规则（不引入 regex，见 02 章 §5.3 拒绝清单）。
pub fn scrub(input: &str) -> String {
    const SENSITIVE: [&str; 8] = [
        "token", "access_token", "api_key", "key", "secret", "passwd", "password", "auth",
    ];
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    'outer: while !rest.is_empty() {
        // 找最近的一个敏感键（后面紧跟 = 或 : 或 %3D）
        let mut best: Option<(usize, usize)> = None; // (pos, key_len)
        for kw in SENSITIVE {
            let mut from = 0usize;
            while let Some(rel) = rest[from..].to_ascii_lowercase().find(kw) {
                let pos = from + rel;
                let after = &rest[pos + kw.len()..];
                let is_kv = after.starts_with('=')
                    || after.starts_with(':')
                    || after.starts_with("%3D")
                    || after.starts_with("%3d");
                // 键边界：前面不能是字母数字（避免 "monkey=" 命中 "key"）
                let prev_ok = pos == 0
                    || !rest.as_bytes()[pos - 1].is_ascii_alphanumeric();
                if is_kv && prev_ok {
                    best = Some((pos, kw.len()));
                    break;
                }
                from = pos + kw.len();
            }
            if best.is_some() {
                break;
            }
        }

        let Some((pos, klen)) = best else {
            out.push_str(rest);
            break 'outer;
        };
        out.push_str(&rest[..pos + klen]);
        let tail = &rest[pos + klen..];
        // 复制分隔符
        let sep_len = if tail.starts_with("%3D") || tail.starts_with("%3d") { 3 } else { 1 };
        out.push_str(&tail[..sep_len.min(tail.len())]);
        let after_sep = &tail[sep_len.min(tail.len())..];
        // 吃掉值，直到 & ? # 空格 " ' 或行尾
        let end = after_sep
            .find(|c: char| matches!(c, '&' | '?' | '#' | ' ' | '"' | '\'' | '\n' | '\r' | '/'))
            .unwrap_or(after_sep.len());
        out.push_str("******");
        rest = &after_sep[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn redacts_query_token() {
        let s = scrub("GET https://a.com/sub?token=abcd1234&flag=1 failed");
        assert!(s.contains("token=******"), "{s}");
        assert!(!s.contains("abcd1234"));
        assert!(s.contains("flag=1"), "非敏感参数应保留: {s}");
    }

    #[test]
    fn redacts_colon_style() {
        let s = scrub("secret: 0123456789abcdef");
        assert!(!s.contains("0123456789abcdef"), "{s}");
    }

    #[test]
    fn does_not_break_plain_words() {
        // "monkey=" 中的 "key" 不应被当成键
        let s = scrub("monkey=banana");
        assert_eq!(s, "monkey=banana");
    }
}
```

**`url_hint` 生成规则（实现 + 示例）**

```rust
// src-tauri/src/subscription/manager.rs（节选）
/// 从完整 URL 生成脱敏 hint：scheme://host/path前段/******
pub fn make_url_hint(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else {
        return "******".to_string();
    };
    let host = u.host_str().unwrap_or("unknown");
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    // path 只保留第一段（通常是 /api 或 /sub），其余视为 token
    let first_seg = u
        .path_segments()
        .and_then(|mut s| s.next())
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    if first_seg.is_empty() {
        format!("{}://{}{}/******", u.scheme(), host, port)
    } else {
        format!("{}://{}{}/{}/******", u.scheme(), host, port, first_seg)
    }
}
```

| 输入 | 输出 |
| --- | --- |
| `https://api.example.com/api/v1/client/subscribe?token=abc123` | `https://api.example.com/api/******` |
| `https://sub.example.com/xxxxx` | `https://sub.example.com/xxxxx/******` |
| `https://example.com` | `https://example.com/******` |
| `not-a-url` | `******` |

---

## 8. 真实样本与手工验证

### 8.1 用本机代理实测（PowerShell）

```powershell
# 1) 只看响应头（验证 subscription-userinfo 是否存在、字段名大小写）
try {
  $r = Invoke-WebRequest -Uri 'https://<your-sub-host>/api/v1/client/subscribe?token=<TOKEN>' `
        -UserAgent 'clash.meta/1.19.0' `
        -Proxy 'http://127.0.0.1:1886' `
        -MaximumRedirection 10 -TimeoutSec 20
  $r.StatusCode
  $r.Headers.GetEnumerator() | Where-Object { $_.Key -match 'userinfo|etag|last-modified|content-length|content-type|profile' } |
      Format-Table -AutoSize -Wrap
  # 注意：控制台会打印 token，验证完请清屏，不要贴到 issue/日志
} catch { $_.Exception.Message }

# 2) 验证条件请求（第二次带 ETag 应返回 304）
curl.exe -s -D - -o NUL --proxy http://127.0.0.1:1886 `
  -A 'clash.meta/1.19.0' `
  -H 'If-None-Match: "<paste-etag-here>"' `
  'https://<your-sub-host>/api/v1/client/subscribe?token=<TOKEN>'

# 3) 验证 8 MiB 上限（构造超大响应体，应被中止）
curl.exe -s --proxy http://127.0.0.1:1886 -o NUL -w "%{size_download}\n" <超大订阅URL>
```

### 8.2 典型响应头样本（脱敏）

```http
HTTP/1.1 200 OK
content-type: text/yaml; charset=utf-8
content-disposition: attachment; filename*=utf-8''MyAirport.yaml
profile-update-interval: 24
profile-web-page-url: https://panel.example.com
subscription-userinfo: upload=1234567890; download=98765432100; total=1099511627776; expire=1767225600
etag: "8f1c2a7e5b9d"
last-modified: Wed, 05 Nov 2026 03:12:44 GMT
content-length: 84213
```

解析结果（期望）：

```json
{
  "upload": 1234567890,
  "download": 98765432100,
  "total": 1099511627776,
  "expireAt": 1767225600,
  "webPageUrl": "https://panel.example.com",
  "source": "Header"
}
```

派生展示 —— **不属本章职责**（见下方「UserInfo 的落点」）：
- 已用 = `1234567890 + 98765432100` = `99,999,999,990` = **93.13 GB**
- 剩余 = `1099511627776 - 99999999990` = `999,511,627,786` = **930.86 GB**
- 使用率 = `9.10%`
- 到期 = `1767225600` → **2026-01-01**（本地时区日期）

> 这些数字仅用于人工核对解析结果是否正确。**代码里不要重算一遍。**

### 8.2.1 `UserInfo` 的落点与派生规则的唯一归属（C.5 v1.1/v1.2）

本章只负责**产出** `UserInfo`（原始量），**不负责**派生量。归属必须清晰，否则会出现「前端算一遍、后端算一遍，两个数不一致」的经典 bug。

| 数据 | 谁产出 | 存在哪 | 谁消费 |
| --- | --- | --- | --- |
| `UserInfo.upload/download/total/expire_at` | **本章** `userinfo.rs`（原始量） | `profiles/<uid>.meta.json` + `Subscription.mgmt` | `traffic_summary` |
| `SubscriptionTraffic.used / used_percent / days_left / stale` | **`traffic_summary` 命令**（C.6）按 C.5 派生规则**一次算好** | 不落盘（每次调用现算） | 前端**只展示** |
| `AggregateTraffic.*` | 同上（聚合规则见 C.5 `TrafficSummary` 注释） | 不落盘 | 前端只展示 |
| 展示格式化（`93.13 GB`、`2026-01-01`） | 前端 `ui/format.ts` | — | UI |

**因此本章的明确禁止项**：

| 禁止 | 原因 |
| --- | --- |
| 在 `userinfo.rs` 里计算 `used` / `used_percent` / `days_left` 并写进 `UserInfo`（**契约没有这些字段**） | 加字段 = 契约变更；且 C.5 规定派生由 `traffic_summary` 统一算 |
| 在 `parse.rs` / `fetch.rs` 里做「剩余流量」逻辑 | 同上 |
| 前端自己用 `upload + download` 再算一遍 `used` | 与后端结果可能因「缺失项按 0 参与求和但 `has_missing=true`」的规则不同而分叉 |
| 把「剩余天数」负数直接显示为 `-3` | C.5：负数表示已过期，UI 文案为「已过期 3 天」 |

**聚合语义提醒**（C.5 `TrafficSummary` 注释原文）：`total` 取各订阅之和（`0` 视为不限量则忽略该订阅）；
`expire_at` 取**最早**的非 `None`；`upload`/`download` 缺失项按 `0` 求和但 `has_missing = true`。
本章的解析只需保证「缺失就是 `None`」，**不要**为了聚合方便提前把 `None` 填成 `0` —— 一旦填 0，
`has_missing` 与 `used: Option<i64>` 就永久失去信息，UI 无法显示 `—`。

**`stale` 的判定输入**：`stale = now - UserInfo.fetched_at > 2 * Subscription.update_interval_min * 60`
（C.5 原文「距 `fetched_at` 超过 2 个更新周期」）。因此本章**必须**正确填 `fetched_at = now`（Unix 秒，C.10），
否则 `stale` 永久为 `true`。`userinfo_from_headers(headers, now)` 的 `now` 参数就是为此存在。

### 8.2.2 首次启动的合规提示（`compliance_accepted_at`）

`AppSettings.compliance_accepted_at`（C.5 v1.1，`schema_version` 已升到 **2**）**不属本章的解析范围**，
但与本章的「首次拉取」时序耦合，此处只做交割说明：

| 项 | 规定 |
| --- | --- |
| 字段 | `AppSettings.compliance_accepted_at: Option<i64>`，`None` = 用户尚未接受合规提示 |
| 何时写 | 仅当用户在首启模态里点「我已了解」时写入 `now`（Unix 秒） |
| 与本章的关系 | **合规提示未接受前，不得发起任何订阅拉取**（否则等于「未经告知就开始联网」） |
| 谁负责 | 08 章（启动流程）+ 09 章（模态 UI）；`commands/settings.rs` 落地 |
| 本章的约束 | `subscription_add` / `subscription_refresh` 的**参数校验阶段**（`commands/` 薄层）必须先检查 `compliance_accepted_at.is_some()`，否则返回 `E_BUSY`「请先确认合规提示」。**不要**在 `fetch.rs` 里做这个检查（层次错误） |
| schema 迁移 | `store/migrate.rs` 必须能把 `schema_version = 1` 的 `settings.json` 升到 2：`compliance_accepted_at = None`（**不要**默认填 `now` —— 那等于替用户同意） |

> **`profile-update-interval`（小时）怎么处理 —— 已定稿（v1.4 Lead 裁决）**：
> **以 `settings.sub_default_interval_min` 为准，我们的生成器不写 `profile-update-interval`。**
> 机场头里的值（样本为 `24` 小时）只在 UI 上作为「机场建议 24 小时」的提示文案展示，**不参与调度**。
>
> 理由：内核的 `profile` 自动更新我们**完全不用**（`profile-update-interval` 是给内核自己定时拉订阅的），
> 若写进 config 就会出现**第二个更新者**——内核与我们各拉一次，既浪费流量，又可能让
> `profiles/<uid>.yaml` 与内核内存里的配置版本不一致。**单一更新者**是本章的硬原则。
>
> 因此：`runtime/config.yaml` 的 `profile:` 段只有 `store-selected` / `store-fake-ip`，
> **不得**出现 `profile-update-interval` / `profile-web-page-url`（后者由我们自己的 `web_page_url` 字段承载）。

### 8.3 手工验收步骤（12 章矩阵的本章切片）

| # | 操作 | 期望 |
| --- | --- | --- |
| 1 | 添加一个正常订阅 | 10 s 内节点数与流量/到期显示完整 |
| 2 | 断网后点刷新 | 中文提示「无法解析订阅域名…」，订阅仍在，旧节点仍可用 |
| 3 | 订阅 URL 填错（404） | 「订阅服务器返回 HTTP 404」，**不**重试 4 次（应只有 1 次请求） |
| 4 | 订阅返回 HTML | 「订阅内容不是有效的 Clash 配置」，本地旧 yaml 未被覆盖 |
| 5 | 订阅返回 `proxies: []` | 「订阅中没有可用节点…」 |
| 6 | 服务器不支持 ETag | 每次都正常全量拉取，无警告刷屏 |
| 7 | 服务器返回 304 | 不重写 yaml；`last_update_at` 更新；UI 无闪烁 |
| 8 | 把 `sub_fetch_proxy` 指向不存在的端口 | 「拉取代理不可用」，且**不**回退直连 |
| 9 | 抓 `logs/app-*.log` 全文搜索 `token=` | 无明文命中 |
| 10 | 导出诊断包 | 包内 `index.json` 的 url 均为 `******` 形式 |
| 11 | 手工把 `profiles/<uid>.yaml` 改成乱码后刷新 | 覆盖为正确内容（**注**：这是用户主动刷新，允许覆盖；但若拉取失败则不动） |
| 12 | 5 个订阅同时 `subscription_refresh_all` | 并发不超过 2，无 `E_BUSY` 误报，总时长 < 40 s |

---

## 9. 本章自检清单（Agent 必须逐条确认）

- [ ] 1. `reqwest::Client` 是否**只在** `util/net.rs` 构造一次并被 `Arc` 复用？有无「每个订阅 new 一个 Client」？
- [ ] 2. UA 是否取自 `settings.sub_user_agent`（默认 `clash.meta/1.19.0`）？有无硬编码浏览器 UA？
- [ ] 3. 连接超时 10 s、总超时取 `sub_fetch_timeout_ms`、重试 3 次且退避为 **1s/2s/4s + `0..300ms` 抖动**？
- [ ] 4. 是否发送 `If-None-Match` / `If-Modified-Since`？304 时是否**不重写** yaml、且本地 yaml 缺失时会重发无条件请求？
- [ ] 5. 响应体是否用 `chunk()` 流式累计并强制 **8 MiB** 上限？有无 `resp.bytes()` 直读？
- [ ] 6. 代理语义是否严格：`None` = 直连（`no_proxy()` 且不读环境变量）；`Some` = 必走代理；代理失败返回 `E_NET_PROXY` 且**不**回退？
- [ ] 7. 是否先解析成功再落盘？落盘顺序是否 yaml → meta → index，且全部走原子写（`*.tmp` → fsync → rename）？
- [ ] 8. YAML 校验是否只把 L0（不可解析 / 无 `proxies` / 节点 0）判失败，L1/L2 仅告警？
- [ ] 9. `parse_userinfo_header` 是否满足 §4.3 全部 15 条用例（尤其：缺字段 → `None` 而非 0、`total=0` → `Some(0)`、毫秒 `expire` 被除以 1000、多余分号、未知键忽略）？
- [ ] 10. 错误码是否与 C.8 **逐字**一致？`retryable` 是否与 §5 表一致？`last_error` 是否为 `"{code}: {message}"` 且**不含** detail？
- [ ] 11. 日志是否只记 `uid/host/status/elapsed_ms/bytes`？是否有任何位置会打印完整 URL、token、secret、节点密码？
- [ ] 12. `url_hint` 是否符合 §7 规则（query 丢弃、token 段替换）？`profile-web-page-url` 是否做了 scheme 白名单校验？
- [ ] 13. 是否**没有**修改 `Subscription` / `UserInfo` 的字段（契约 C.5）？如需展示新信息，是否从内存投影或 meta 读取而非加字段？
- [ ] 14. 错误 detail 与 panic 信息是否都过 `redact::scrub()`？`scrub` 是否有 `monkey=` 这类误伤用例的单测？
- [ ] 15. `E_USERINFO_MISSING` 出现时是否**不**把订阅标记为 Failed（订阅本身可用）？
- [ ] 16. 网页兜底是否**确实没有实现**（MVP 不提供，`TODO(待确认)`）？`userinfo.rs` 里没有任何 HTTP 抓 HTML 的代码路径？若将来启用，是否先走了 `CONTRACT-CHANGE-REQUEST:` 并新增 `sub_userinfo_web_fallback` 字段？
- [ ] 17. 损坏文件是否按 §6.3 处置（`*.corrupt-<ts>` 备份 + 默认值 + 中文提示），且**不崩溃**？
- [ ] 18. 本章是否**未**修改 `01`/`C`/其它章节？若有契约缺口，是否以 `CONTRACT-CHANGE-REQUEST:` 提出？
- [ ] 19. 是否**没有**在 `userinfo.rs` / `parse.rs` / `fetch.rs` 里计算 `used` / `used_percent` / `days_left`？（这些只能由 `traffic_summary` 按 C.5 派生）
- [ ] 20. 是否**没有**把 `UserInfo` 的 `None` 填成 `0`？`total == 0` 是否仍保留为 `Some(0)`（「不限量」）？
- [ ] 21. `fetched_at` 是否填入真实 `now`（Unix 秒）？有无留 0 导致 `stale` 恒为 `true`？
- [ ] 22. 合规检查（`compliance_accepted_at.is_some()`）是否在 `commands/` 薄层？`fetch.rs` 里**没有**做该判断？
- [ ] 23. `store/migrate.rs` 的 1→2 迁移是否把 `compliance_accepted_at` 置为 `None`（而**不是** `now`）？

---

## 10. 契约对齐记录（v1.3）

> 本章原按 v1.2 落笔；C 章升到 **v1.3**（§C.6.1 内部接口冻结面）后复核：
> **本章不依赖 v1.3 的任何新增类型**（订阅域不接触 `SelectorApi`/`ApiError`/组结构），
> 因此在 v1.3 下无需代码级改动，仅补充下表第 9–11 行的复核结论。

| # | 本章做法 | 对应契约 |
| --- | --- | --- |
| 1 | 新增 §8.2.1：明确 `UserInfo` 只承载**原始量**，`used`/`used_percent`/`days_left`/`stale` 由 `traffic_summary` 命令按 C.5 派生规则**一次算好**；显式列出「本章禁止重算派生量」的 4 条禁止项 | C.5 v1.1 `TrafficSummary` / `SubscriptionTraffic` / `AggregateTraffic` 注释 |
| 2 | 解析结果**不得**把 `None` 填成 `0`（否则 `has_missing` 与 `used: Option<i64>` 失去信息） | C.5 v1.1 聚合规则「缺失项按 0 参与求和但 `has_missing` 置 true」 |
| 3 | 明确 `fetched_at` 必须填 `now`（Unix 秒），否则 `stale` 永久为 `true` | C.5 v1.1 `SubscriptionTraffic.stale` + C.10 |
| 4 | 新增 §8.2.2：`compliance_accepted_at`（`schema_version` → **2**）。合规提示未接受前**不得发起订阅拉取**；检查点在 `commands/` 薄层而非 `fetch.rs`；迁移时默认 `None`（不得替用户同意） | C.5 v1.1 `AppSettings.compliance_accepted_at`、C.12.6 |
| 5 | 本章**不新增**任何错误码：所有失败复用 C.8 的 `E_NET_*` / `E_HTTP_STATUS` / `E_PARSE_*` / `E_STORE_*` / `E_USERINFO_MISSING` / `E_BUSY` | C.8 |
| 6 | 网页兜底结果 `source = WebPage`，且**不参与**流量告警（只有 `Header` 来源可触发 `traffic_warn_percent`） | C.5 `UserInfoSource`、C.5 派生规则 |
| 7 | 未改动 `Subscription` / `UserInfo` 的字段；需要展示的新信息走内存投影或 meta 读取 | C.5（本章 §3.3 明确禁止加字段） |
| 8 | 03 章未受 `switch_apply_best` / `SwitchRecord.to_delay` / `manual_override_min`（v1.2）影响：订阅域不感知切换语义 | C.6 v1.2、C.5 v1.2 |
| 9 | **（v1.3 复核）** 本章**不使用** `SelectorApi` / `ApiError` / `GroupInfo` / `SelectableGroup` / `SwitchRuntimeLocks` 任何一个冻结类型；`subscription/manager.rs` 只调 `kernel/api.rs` 的 `PUT /configs`（04 章 §8），不碰 selector 接口 | C.6.1 |
| 10 | **（v1.3 复核）** 本章产出的 `node_key` 是 04 章合并结果，最终会成为 `/proxies/{name}` 的 `{name}`；因此含 `/` 等字符时必须 URL-encode（已在 04 章 §4.1 明确，本章不重复定义） | C.6.1 `kind` 陷阱（反向：`node_key` 是 API 路径参数，非 `kind`） |
| 11 | **（本轮自查）** §2.6 删除 `bytes` / `futures` 依赖：只用 `Response::chunk()` 流式读取，二者均不在 02 章 §5.1 白名单 | 01 铁律 5（依赖白名单） |
| 12 | **（v1.4 对齐）** `profile-update-interval` **不使用**：生成器不写该键，调度周期一律以 `settings.sub_default_interval_min` 为准，机场值仅作 UI 提示文案。避免出现「内核 + 本应用」两个订阅更新者 | Lead v1.4 裁决、附录 A（`profile:` 段） |
| 13 | **（v1.4 对齐）** 网页兜底在 MVP **不实现**（`TODO(待确认)`），不新增 `sub_userinfo_web_fallback` 字段；头缺失即 `E_USERINFO_MISSING` 静默降级，不做任何网页抓取 | Lead v1.4 裁决（该字段不进契约） |

### 已裁决事项存档（v1.4 Lead 确认，本章无需再改）

| # | 原未决项 | 裁决 | 本章落地 |
| --- | --- | --- | --- |
| 1 | `profile-update-interval` 与 `sub_default_interval_min` 优先级 | **以 `sub_default_interval_min` 为准**；生成器**不写** `profile-update-interval`（避免第二个更新者） | §8.2 已改写为定稿说明；`profile:` 段只含 `store-selected` / `store-fake-ip` |
| 2 | `sub_userinfo_web_fallback` 是否新增字段 | **不进契约**，MVP 不做网页兜底 | §4.5 标题改为「MVP 不实现，仅保留设计」并标 `TODO(待确认)`；自检项 16 相应调整 |
| 3 | `E_USERINFO_MISSING` 是否写 `Subscription.last_error` | **不写**，保持 `last_update_status = Ok`（流量信息缺失不是订阅失败） | 本章原实现即如此，无需改动；理由已补记：写进 `last_error` 会让 UI 显示「更新失败」红点，误导用户 |

> **本章无遗留未决项。**

---
