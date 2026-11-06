//! 脱敏工具（契约 C.9）。日志与诊断导出**必须**过一遍这里。

use once_cell::sync::Lazy;
use regex_lite::Regex;

/// URL 中的 token/key/password 等查询参数
static QUERY_SECRET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)([?&](?:token|key|apikey|api_key|password|passwd|pwd|secret|auth|access_token|subscribe|sub)=)([^&#\s]+)",
    )
    .expect("正则合法")
});

/// `scheme://user:pass@host` 形式的凭据
static URL_USERINFO: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(://)([^/@\s:]+):([^/@\s]+)@").expect("正则合法"));

/// 订阅路径尾段（很多机场把 token 直接放 path）
static PATH_TOKEN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(/api/v\d/client/subscribe/)([A-Za-z0-9_\-]{8,})").expect("正则合法"));

/// Authorization 头
static BEARER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(bearer\s+)([A-Za-z0-9._\-]+)").expect("正则合法"));

/// JSON 里的敏感字段
static JSON_SECRET: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)("(?:password|passwd|secret|token|uuid|private_key|psk)"\s*:\s*")([^"]*)(")"#)
        .expect("正则合法")
});

/// 掩码占位
pub const MASK: &str = "******";

/// 通用脱敏：处理 URL 查询参数、userinfo、path token、Bearer、JSON 字段
pub fn sanitize(input: &str) -> String {
    let s = QUERY_SECRET.replace_all(input, format!("${{1}}{MASK}").as_str());
    let s = URL_USERINFO.replace_all(&s, format!("${{1}}{MASK}:{MASK}@"));
    let s = PATH_TOKEN.replace_all(&s, format!("${{1}}{MASK}"));
    let s = BEARER.replace_all(&s, format!("${{1}}{MASK}"));
    let s = JSON_SECRET.replace_all(&s, format!("${{1}}{MASK}${{3}}"));
    s.into_owned()
}

/// 生成展示用 URL 提示：保留 scheme + host + 前若干字符路径，其余打码。
///
/// 目标是"用户能认出是哪个订阅"，但**不泄露可用于访问的完整地址**。
pub fn url_hint(raw: &str) -> String {
    let Ok(u) = url::Url::parse(raw) else {
        return MASK.to_string();
    };
    let scheme = u.scheme();
    let host = u.host_str().unwrap_or("");
    let path = u.path();

    // 路径只保留前 2 段，尾段（常为 token）打码
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let shown = if segs.len() > 2 {
        format!("/{}/{}/{}", segs[0], segs[1], MASK)
    } else if segs.is_empty() {
        String::new()
    } else {
        format!("/{}/{}", segs[0], MASK)
    };

    format!("{scheme}://{host}{shown}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_query_token() {
        let s = "https://a.com/sub?token=SECRET123&name=x";
        let out = sanitize(s);
        assert!(!out.contains("SECRET123"), "token 必须打码: {out}");
        assert!(out.contains("name=x"), "非敏感参数保留");
    }

    #[test]
    fn masks_userinfo_password() {
        let out = sanitize("http://user:pass@127.0.0.1:1886");
        assert!(!out.contains(":pass@"), "密码必须打码: {out}");
    }

    #[test]
    fn masks_subscribe_path_token() {
        let out = sanitize("https://airport.com/api/v1/client/subscribe/abcdef1234567890");
        assert!(!out.contains("abcdef1234567890"), "路径 token 必须打码: {out}");
    }

    #[test]
    fn masks_bearer_header() {
        let out = sanitize("Authorization: Bearer 3f9c0a7e5b2d4c8f1a6e9b0d3c7f5a21");
        assert!(!out.contains("3f9c0a7e5b2d4c8f"), "Bearer 必须打码: {out}");
    }

    #[test]
    fn masks_json_secrets() {
        let out = sanitize(r#"{"password":"hunter2","name":"ok"}"#);
        assert!(!out.contains("hunter2"), "JSON 密码必须打码: {out}");
        assert!(out.contains("\"name\":\"ok\""), "非敏感字段保留");
    }

    #[test]
    fn keeps_normal_text_intact() {
        let s = "节点 HongKong-01 延迟 45ms";
        assert_eq!(sanitize(s), s, "普通文本不应被改动");
    }

    #[test]
    fn url_hint_hides_token_segment() {
        let h = url_hint("https://airport.com/api/v1/client/subscribe/abcdef123456");
        assert!(h.contains("airport.com"), "应保留 host 便于识别: {h}");
        assert!(!h.contains("abcdef123456"), "尾段必须打码: {h}");
    }

    #[test]
    fn url_hint_invalid_url_is_masked() {
        assert_eq!(url_hint("not a url"), MASK);
    }
}
