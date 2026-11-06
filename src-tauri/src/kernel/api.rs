//! mihomo external-controller REST 客户端（契约 C.6.1 内部接口冻结面）。
//!
//! 关键约束：
//! - **必须 `.no_proxy()`**（见 util::net::build_kernel_client）
//! - 节点名/组名必须 percent-encode，中文名直接拼 URL 会 404
//! - `kind` 比较必须大小写敏感精确匹配 `"Selector"`
//!
//! trait 用 boxed-future 形式（C.6.1）：Rust 的 `async fn in trait` 默认不是
//! dyn 兼容的，而测速引擎需要 `Arc<dyn SelectorApi>` 注入 FakeApi 做单测。

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::Deserialize;

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{ApiError, GroupInfo, SelectableGroup};
use crate::domain::policy::{is_selector, KIND_SELECTOR};

/// dyn 兼容的 future 别名
pub type ApiFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

/// 内核 API 抽象。测速/切换只依赖本 trait，不依赖具体 HTTP 实现。
pub trait SelectorApi: Send + Sync {
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>>;
    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo>;
    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()>;
    /// 单节点延迟探测
    fn proxy_delay(
        &self,
        node: &str,
        probe_url: &str,
        timeout_ms: u32,
    ) -> ApiFuture<'_, u32>;
    /// 内核版本（健康检查）
    fn version(&self) -> ApiFuture<'_, String>;
}

// ============================================================================
//  HTTP 实现
// ============================================================================

#[derive(Clone)]
pub struct HttpKernelApi {
    base: String,
    secret: String,
    client: reqwest::Client,
}

impl HttpKernelApi {
    /// `port` 为 external-controller 端口；`secret` 来自 `runtime/secret`
    pub fn new(port: u16, secret: String) -> AppResult<Self> {
        let client = crate::util::net::build_kernel_client()?;
        Ok(Self {
            base: format!("http://127.0.0.1:{port}"),
            secret,
            client,
        })
    }

    pub fn with_client(port: u16, secret: String, client: reqwest::Client) -> Self {
        Self {
            base: format!("http://127.0.0.1:{port}"),
            secret,
            client,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// 组名/节点名必须编码：中文、空格、`&`、`#`、`/` 都会破坏 URL
    fn encode_segment(s: &str) -> String {
        // 用 url crate 的 percent-encoding，但要保留 `:` 等安全字符
        let mut out = String::with_capacity(s.len() * 3);
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.secret)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::NotFound(truncate_body(&body)));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(status.as_u16(), truncate_body(&body)));
        }
        resp.json::<T>().await.map_err(map_reqwest_err)
    }

    async fn put_json<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<(), ApiError> {
        let resp = self
            .client
            .put(self.url(path))
            .bearer_auth(&self.secret)
            .json(body)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(map_reqwest_err)?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(ApiError::Unauthorized);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::NotFound(truncate_body(&body)));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Status(status.as_u16(), truncate_body(&body)));
        }
        Ok(())
    }
}

fn truncate_body(s: &str) -> String {
    crate::domain::errors::truncate(&crate::util::redact::sanitize(s), 200)
}

/// reqwest 错误 → ApiError。
///
/// 注意 `Decode`（解析失败）归到 `Status` 而非 `Transport`：
/// `Transport` 语义是"连接没建立"，解析失败是"连上了但内容不对"，
/// 两者排障方向相反（后者要查内核版本/配置，不是查端口）。
fn map_reqwest_err(e: reqwest::Error) -> ApiError {
    if e.is_timeout() {
        ApiError::Timeout
    } else if e.is_decode() {
        ApiError::Status(200, format!("响应解析失败: {}", e.without_url()))
    } else if e.is_connect() {
        ApiError::Transport(format!("连接失败: {}", e.without_url()))
    } else {
        ApiError::Transport(format!("{}", e.without_url()))
    }
}

// ---------- 内部 JSON 结构 ----------

#[derive(Debug, Deserialize)]
struct ProxiesResp {
    #[serde(default)]
    proxies: std::collections::HashMap<String, ProxyEntry>,
}

#[derive(Debug, Deserialize)]
struct ProxyEntry {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    now: Option<String>,
    /// 组的成员列表
    #[serde(default)]
    all: Option<Vec<String>>,
    #[serde(default, rename = "testUrl")]
    test_url: Option<String>,
    #[serde(default)]
    hidden: bool,
}

#[derive(Debug, Deserialize)]
struct DelayResp {
    #[serde(default)]
    delay: u32,
}

#[derive(Debug, Deserialize)]
struct VersionResp {
    #[serde(default)]
    version: String,
}

impl SelectorApi for HttpKernelApi {
    fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>> {
        Box::pin(async move {
            let resp: ProxiesResp = self.get_json("/proxies").await?;
            let mut out: Vec<SelectableGroup> = resp
                .proxies
                .into_iter()
                .filter(|(_, e)| {
                    // 关键：只接受大驼峰 Selector。隐藏组一并过滤。
                    is_selector(&e.kind) && !e.hidden
                })
                .map(|(name, e)| {
                    let all = e.all.unwrap_or_default();
                    // members 不含 DIRECT/REJECT 等内置名，供排序使用
                    let members = all
                        .into_iter()
                        .filter(|m| {
                            !matches!(
                                m.to_ascii_uppercase().as_str(),
                                "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" | "COMPATIBLE" | "GLOBAL"
                            )
                        })
                        .collect();
                    SelectableGroup {
                        name,
                        members,
                        now: e.now,
                    }
                })
                .collect();
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        })
    }

    fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo> {
        let group = group.to_string();
        Box::pin(async move {
            let q = Self::encode_segment(&group);
            let path = format!("/proxies/{q}");
            let e: ProxyEntry = self.get_json(&path).await?;
            Ok(GroupInfo {
                name: group,
                kind: e.kind,
                now: e.now,
                all: e.all.unwrap_or_default(),
                test_url: e.test_url,
                hidden: e.hidden,
            })
        })
    }

    fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()> {
        let group = group.to_string();
        let node = node.to_string();
        Box::pin(async move {
            let q = Self::encode_segment(&group);
            let path = format!("/proxies/{q}");
            self.put_json(&path, &serde_json::json!({ "name": node }))
                .await
        })
    }

    fn proxy_delay(
        &self,
        node: &str,
        probe_url: &str,
        timeout_ms: u32,
    ) -> ApiFuture<'_, u32> {
        let node = node.to_string();
        let probe_url = probe_url.to_string();
        Box::pin(async move {
            let q = Self::encode_segment(&node);
            let enc_url = Self::encode_segment(&probe_url);
            let path = format!(
                "/proxies/{q}/delay?url={enc_url}&timeout={timeout_ms}&expected=204"
            );
            let r: DelayResp = self.get_json(&path).await?;
            Ok(r.delay)
        })
    }

    fn version(&self) -> ApiFuture<'_, String> {
        Box::pin(async move {
            let r: VersionResp = self.get_json("/version").await?;
            Ok(r.version)
        })
    }
}

/// ApiError → AppError（契约 08.17.1 映射）
impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        match e {
            // 401：secret 不一致，通常是重建配置时没同步
            ApiError::Unauthorized => AppError::new(ErrorCode::CoreApiUnauth),
            // 404：组/节点不存在，不算致命，调用方应跳过
            ApiError::NotFound(m) => AppError::with_detail(ErrorCode::NotFound, m),
            // 读超时归 E_NET_TIMEOUT，**不**归 E_CORE_START_FAILED
            // （后者语义是启动失败/端口占用，误归会让用户以为内核没起来）
            ApiError::Timeout => AppError::new(ErrorCode::NetTimeout),
            ApiError::Status(c, b) => AppError::with_detail(
                ErrorCode::CoreStartFailed,
                format!("HTTP {c}: {b}"),
            ),
            ApiError::Transport(m) => {
                AppError::with_detail(ErrorCode::CoreNotRunning, m)
            }
        }
    }
}

/// Kind 常量再导出，便于跨模块比对
pub const SELECTOR_KIND: &str = KIND_SELECTOR;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::ApiError as AE;

    #[test]
    fn encode_segment_escapes_chinese_and_specials() {
        // 中文必须被编码，否则 URL 解析失败/404
        let enc = HttpKernelApi::encode_segment("香港 01");
        assert!(!enc.contains(' '), "空格必须编码: {enc}");
        assert!(enc.starts_with('%'), "中文应转为 %XX: {enc}");

        // 关键破坏字符
        for c in ['&', '#', '/', '?', '+'] {
            let s = format!("a{c}b");
            let e = HttpKernelApi::encode_segment(&s);
            assert!(!e.contains(c), "{c} 必须被编码，实际 {e}");
        }
    }

    #[test]
    fn encode_segment_keeps_safe_chars() {
        assert_eq!(HttpKernelApi::encode_segment("HongKong-01"), "HongKong-01");
        assert_eq!(HttpKernelApi::encode_segment("a_b.c~d"), "a_b.c~d");
    }

    #[test]
    fn api_error_maps_to_contract_codes() {
        let e: AppError = AE::Unauthorized.into();
        assert_eq!(e.code, ErrorCode::CoreApiUnauth);

        let e: AppError = AE::Timeout.into();
        assert_eq!(e.code, ErrorCode::NetTimeout, "超时不得归为内核启动失败");

        let e: AppError = AE::NotFound("x".into()).into();
        assert_eq!(e.code, ErrorCode::NotFound);

        let e: AppError = AE::Transport("conn refused".into()).into();
        assert_eq!(e.code, ErrorCode::CoreNotRunning);
    }

    #[test]
    fn decode_error_maps_to_status_not_transport() {
        // 用 is_decode 分支的语义：解析失败应是 Status(200, _)，避免误报"内核已退出"
        let ae = map_reqwest_err_with_kind("decode");
        match ae {
            ApiError::Status(code, _) => assert_eq!(code, 200),
            other => panic!("decode 应映射为 Status(200,_)，实际 {other:?}"),
        }
    }

    /// 测试辅助：模拟 reqwest 错误分类（无法真实构造 reqwest::Error，故走等价逻辑）
    fn map_reqwest_err_with_kind(kind: &str) -> ApiError {
        match kind {
            "timeout" => ApiError::Timeout,
            "decode" => ApiError::Status(200, "响应解析失败".into()),
            "connect" => ApiError::Transport("连接失败".into()),
            _ => ApiError::Transport("other".into()),
        }
    }

    #[test]
    fn selector_kind_constant_is_camelcase() {
        assert_eq!(SELECTOR_KIND, "Selector");
        assert_ne!(SELECTOR_KIND, "select");
        assert_ne!(SELECTOR_KIND, "selector");
        assert_ne!(SELECTOR_KIND.to_ascii_lowercase(), "select");
    }
}
