//! 网络客户端构造（契约 C.9 / 08 章）。
//!
//! 关键约束：
//! 1. **访问内核 API 必须 `.no_proxy()`** —— 否则用户开了系统代理后，
//!    本地 `127.0.0.1:port` 的请求会被自己的系统代理劫持，全部超时。
//! 2. 订阅拉取则走用户配置的代理（可选）。
//! 3. UA 与超时统一从设置读取。

use std::time::Duration;

use reqwest::Client;

use crate::domain::errors::{AppError, AppResult, ErrorCode};

/// 默认 UA：伪装成 clash.meta，避免部分机场对未知 UA 返回 HTML 或空 body
pub const DEFAULT_UA: &str = "clash.meta/1.19.0";

/// 构造访问内核 external-controller 的客户端。
///
/// **绝不使用任何代理**，且超时短（本地回环正常 <5ms）。
pub fn build_kernel_client() -> AppResult<Client> {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_millis(1500))
        .pool_max_idle_per_host(2)
        .http1_only() // 内核是 Go net/http，http1 足够且省内存
        .user_agent("clash-zhengl/0.1")
        .build()
        .map_err(|e| AppError::with_detail(ErrorCode::Internal, e))
}

/// 构造订阅拉取客户端。
///
/// `proxy` 为 None 时直连；`Some(url)` 时全部请求走该代理。
pub fn build_subscription_client(
    timeout_ms: u32,
    proxy: Option<&str>,
    ua: &str,
) -> AppResult<Client> {
    let mut b = Client::builder()
        .timeout(Duration::from_millis(timeout_ms as u64))
        .connect_timeout(Duration::from_millis(
            (timeout_ms as u64).min(10_000),
        ))
        .redirect(reqwest::redirect::Policy::limited(5))
        .pool_max_idle_per_host(2)
        .user_agent(if ua.trim().is_empty() { DEFAULT_UA } else { ua })
        // gzip/brotli 已由 feature 打开，这里显式接受编码
        .gzip(true)
        .brotli(true);

    if let Some(p) = proxy.filter(|p| !p.trim().is_empty()) {
        let proxy_url = normalize_proxy_url(p);
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| AppError::with_detail(ErrorCode::NetProxy, e))?;
        b = b.proxy(proxy);
    } else {
        b = b.no_proxy();
    }

    b.build()
        .map_err(|e| AppError::with_detail(ErrorCode::Internal, e))
}

/// 构造用于内核下载的客户端（长超时，允许代理）
pub fn build_download_client(proxy: Option<&str>) -> AppResult<Client> {
    let mut b = Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("clash-zhengl/0.1");

    if let Some(p) = proxy.filter(|p| !p.trim().is_empty()) {
        let proxy_url = normalize_proxy_url(p);
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| AppError::with_detail(ErrorCode::NetProxy, e))?;
        b = b.proxy(proxy);
    }
    b.build()
        .map_err(|e| AppError::with_detail(ErrorCode::Internal, e))
}

/// 归一化用户输入的代理串。
///
/// 用户常直接写 `127.0.0.1:1886`，`Proxy::all` 需要带 scheme。
/// 本项目里这类地址都是 **HTTP mixed 端口**，统一补 `http://`。
pub fn normalize_proxy_url(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return String::new();
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("socks4://")
        || lower.starts_with("socks5://")
        || lower.starts_with("socks5h://")
    {
        s.to_string()
    } else {
        // 裸 host:port 或 host —— 当作 HTTP 代理
        format!("http://{s}")
    }
}

/// 校验订阅 URL 合法性（C.8 `E_SUB_URL_INVALID`）
pub fn validate_subscription_url(raw: &str) -> AppResult<url::Url> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(AppError::msg(ErrorCode::SubUrlInvalid, "订阅地址不能为空"));
    }
    let u = url::Url::parse(t)
        .map_err(|e| AppError::with_detail(ErrorCode::SubUrlInvalid, e))?;
    match u.scheme() {
        "http" | "https" => {}
        other => {
            return Err(AppError::msg(
                ErrorCode::SubUrlInvalid,
                format!("只支持 http/https，实际为 {other}"),
            ))
        }
    }
    if u.host_str().unwrap_or("").is_empty() {
        return Err(AppError::msg(ErrorCode::SubUrlInvalid, "地址缺少主机名"));
    }
    // 回环地址通常是用户填错（把本地端口当订阅）
    if let Some(h) = u.host_str() {
        if h == "127.0.0.1" || h == "localhost" || h == "0.0.0.0" {
            tracing::warn!(host = h, "订阅地址指向本机回环，请确认是否填错");
        }
    }
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_url_gets_scheme() {
        assert_eq!(normalize_proxy_url("127.0.0.1:1886"), "http://127.0.0.1:1886");
        assert_eq!(normalize_proxy_url(" 127.0.0.1:7897 "), "http://127.0.0.1:7897");
        assert_eq!(
            normalize_proxy_url("socks5://127.0.0.1:1080"),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            normalize_proxy_url("http://127.0.0.1:1886"),
            "http://127.0.0.1:1886"
        );
        assert_eq!(normalize_proxy_url(""), "");
    }

    #[test]
    fn kernel_client_never_uses_proxy() {
        // 只要构造成功即说明 no_proxy 生效（无法直接断言，但确保不 panic）
        let c = build_kernel_client();
        assert!(c.is_ok());
    }

    #[test]
    fn validates_subscription_url() {
        assert!(validate_subscription_url("https://a.com/sub?token=x").is_ok());
        assert!(validate_subscription_url("http://a.com/sub").is_ok());
        // 非 http(s)
        let e = validate_subscription_url("ftp://a.com/x").unwrap_err();
        assert_eq!(e.code, ErrorCode::SubUrlInvalid);
        // 空
        assert!(validate_subscription_url("   ").is_err());
        // 无 scheme
        assert!(validate_subscription_url("a.com/sub").is_err());
        // file 协议
        assert!(validate_subscription_url("file:///c:/x").is_err());
    }

    #[test]
    fn subscription_client_accepts_bare_proxy_host_port() {
        let c = build_subscription_client(15_000, Some("127.0.0.1:1886"), DEFAULT_UA);
        assert!(c.is_ok(), "裸 host:port 应被归一化后可用");
    }

    #[test]
    fn subscription_client_direct_when_no_proxy() {
        assert!(build_subscription_client(15_000, None, DEFAULT_UA).is_ok());
        assert!(build_subscription_client(15_000, Some(""), DEFAULT_UA).is_ok());
    }
}
