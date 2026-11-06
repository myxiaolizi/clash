//! 订阅 HTTP 拉取（P2）。
//!
//! 要点：
//! - 条件请求（ETag / If-None-Match / Last-Modified），304 时复用本地缓存
//! - 响应体大小上限，防止机场返回超大文件打爆内存
//! - 流式读取 + 边读边计数，超限立刻中断
//! - UA 可配置（部分机场对 UA 敏感）

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::UserInfoSource;
use crate::subscription::parse::{looks_like_clash_config, parse_subscription, ParsedSubscription};
use crate::subscription::userinfo::{parse_header, to_user_info};
use crate::util::net::{build_subscription_client, validate_subscription_url, DEFAULT_UA};

/// 响应体大小上限：32 MB。
/// 千节点配置约 1-3 MB，32 MB 已能覆盖极端情况，同时防止恶意/异常响应打爆内存。
pub const MAX_BODY_BYTES: u64 = 32 * 1024 * 1024;

/// 一次拉取的结果
#[derive(Debug)]
pub struct FetchOutcome {
    pub parsed: ParsedSubscription,
    pub userinfo: Option<crate::domain::model::UserInfo>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// true = 304，使用本地缓存内容
    pub not_modified: bool,
    pub body_bytes: u64,
    /// 原始配置文本（用于落盘）
    pub raw_text: String,
}

/// 拉取并解析订阅。
///
/// `etag`/`last_modified` 传入上次的值以启用条件请求。
/// 命中 304 时用 `cached_text` 重新解析（保证 meta 里的 etc 与内容一致）。
pub async fn fetch_subscription(
    url: &str,
    proxy: Option<&str>,
    ua: &str,
    timeout_ms: u32,
    etag: Option<&str>,
    last_modified: Option<&str>,
    cached_text: Option<&str>,
) -> AppResult<FetchOutcome> {
    let parsed_url = validate_subscription_url(url)?;
    let client = build_subscription_client(timeout_ms, proxy, if ua.is_empty() { DEFAULT_UA } else { ua })?;

    let mut req = client.get(parsed_url.as_str()).header(
        reqwest::header::ACCEPT,
        "application/yaml, text/yaml, text/plain, */*",
    );
    if let Some(e) = etag.filter(|s| !s.is_empty()) {
        req = req.header(reqwest::header::IF_NONE_MATCH, e);
    }
    if let Some(lm) = last_modified.filter(|s| !s.is_empty()) {
        req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm);
    }

    let resp = req.send().await.map_err(map_net_err)?;

    // 304：内容未变
    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        let text = cached_text
            .ok_or_else(|| {
                AppError::msg(
                    ErrorCode::SubEmpty,
                    "服务器返回 304 但本地无缓存，请强制更新",
                )
            })?
            .to_string();
        let parsed = parse_subscription(&text)?;
        return Ok(FetchOutcome {
            parsed,
            userinfo: None,
            etag: etag.map(|s| s.to_string()),
            last_modified: last_modified.map(|s| s.to_string()),
            not_modified: true,
            body_bytes: text.len() as u64,
            raw_text: text,
        });
    }

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        // 401/403 通常是令牌失效或订阅过期
        let msg = match code {
            401 | 403 => "订阅鉴权失败：令牌可能已失效或订阅已过期",
            404 => "订阅地址不存在（404）",
            429 => "请求过于频繁（429），请调大更新间隔",
            _ => "订阅服务器返回错误状态",
        };
        return Err(AppError::msg(ErrorCode::HttpStatus, format!("{msg}（HTTP {code}）")));
    }

    // 解析 userinfo 头（注意：部分机场在 304 时不带该头，所以只在 200 分支取）
    let userinfo = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_header(Some(s)))
        .map(|p| to_user_info(p, UserInfoSource::Header));

    let etag_resp = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let lm_resp = resp
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // 提前按 Content-Length 拦截超大响应
    if let Some(len) = resp.content_length() {
        if len > MAX_BODY_BYTES {
            return Err(AppError::msg(
                ErrorCode::SubEmpty,
                format!(
                    "订阅内容过大（{:.1} MB），超过 {} MB 上限",
                    len as f64 / 1_048_576.0,
                    MAX_BODY_BYTES / 1_048_576
                ),
            ));
        }
    }

    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = resp.chunk().await.map_err(map_net_err)? {
        buf.extend_from_slice(&chunk);
        if buf.len() as u64 > MAX_BODY_BYTES {
            return Err(AppError::msg(
                ErrorCode::SubEmpty,
                "订阅内容超过大小上限，已中断下载",
            ));
        }
    }

    let body_bytes = buf.len() as u64;
    if buf.is_empty() {
        return Err(AppError::msg(ErrorCode::SubEmpty, "订阅返回内容为空"));
    }

    let text = String::from_utf8_lossy(&buf).into_owned();

    // 早期识别非配置响应，给出准确错误
    if !looks_like_clash_config(&text) {
        return Err(AppError::msg(
            ErrorCode::ParseYaml,
            "返回内容不是 Clash 配置（可能是错误页或需要更换 UA）",
        ));
    }

    let parsed = parse_subscription(&text)?;

    tracing::info!(
        nodes = parsed.node_count(),
        bytes = body_bytes,
        has_userinfo = userinfo.is_some(),
        "订阅拉取成功"
    );

    Ok(FetchOutcome {
        parsed,
        userinfo,
        etag: etag_resp,
        last_modified: lm_resp,
        not_modified: false,
        body_bytes,
        raw_text: text,
    })
}

/// reqwest 错误 → 细分网络错误码，便于 UI 给出可操作提示
fn map_net_err(e: reqwest::Error) -> AppError {
    if e.is_timeout() {
        return AppError::with_detail(ErrorCode::NetTimeout, e.without_url());
    }
    if e.is_connect() {
        let s = e.to_string().to_ascii_lowercase();
        if s.contains("dns") || s.contains("resolve") || s.contains("name or service") {
            return AppError::with_detail(ErrorCode::NetDns, e.without_url());
        }
        if s.contains("tls") || s.contains("certificate") || s.contains("ssl") {
            return AppError::with_detail(ErrorCode::NetTls, e.without_url());
        }
        if s.contains("proxy") {
            return AppError::with_detail(ErrorCode::NetProxy, e.without_url());
        }
        return AppError::with_detail(ErrorCode::NetOffline, e.without_url());
    }
    if e.is_redirect() {
        return AppError::msg(ErrorCode::HttpStatus, "订阅地址重定向次数过多");
    }
    AppError::with_detail(ErrorCode::NetProxy, e.without_url())
}

/// 计算两个时间点之间的抓取间隔是否已到期
pub fn is_due(last: Option<i64>, interval_min: u32, now: i64) -> bool {
    let Some(last) = last else {
        return true; // 从未抓取过
    };
    let interval = (interval_min.max(15) as i64) * 60;
    now.saturating_sub(last) >= interval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_when_never_fetched() {
        assert!(is_due(None, 360, 1000));
    }

    #[test]
    fn due_respects_interval() {
        let now = 10_000;
        // 间隔 15 分钟 = 900 秒
        assert!(!is_due(Some(now - 800), 15, now));
        assert!(is_due(Some(now - 901), 15, now));
    }

    #[test]
    fn due_enforces_minimum_15_minutes() {
        // 即使传入更小的间隔，也应被按 15 分钟处理
        let now = 10_000;
        assert!(!is_due(Some(now - 100), 1, now), "周期下限 15 分钟");
    }

    #[test]
    fn max_body_limit_is_32mb() {
        assert_eq!(MAX_BODY_BYTES, 33_554_432);
    }

    #[test]
    fn invalid_url_rejected_before_request() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let r = rt.block_on(fetch_subscription(
            "not-a-url",
            None,
            "clash.meta/1.19.0",
            5000,
            None,
            None,
            None,
        ));
        assert_eq!(r.unwrap_err().code, ErrorCode::SubUrlInvalid);
    }
}
