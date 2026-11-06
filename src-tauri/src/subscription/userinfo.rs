//! `subscription-userinfo` 响应头解析（附录 B）。
//!
//! 格式：`upload=123; download=456; total=1073741824; expire=1798761600`
//!
//! 机场实现差异极大，必须全部容错：
//! - 字段缺失、顺序不定、多余空格
//! - `total=0` 表示**不限量**（不是"0 字节配额"）
//! - `expire` 有的机场给毫秒，需要按量级判别
//! - 整条头部缺失是常态（返回 None，不算错误）

use crate::domain::model::{UserInfo, UserInfoSource};
use crate::util::time::now_secs;

/// 解析结果
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedUserInfo {
    pub upload: Option<i64>,
    pub download: Option<i64>,
    pub total: Option<i64>,
    pub expire_at: Option<i64>,
}

impl ParsedUserInfo {
    pub fn is_empty(&self) -> bool {
        self.upload.is_none()
            && self.download.is_none()
            && self.total.is_none()
            && self.expire_at.is_none()
    }
}

/// 解析 `subscription-userinfo` 头。缺失或完全无法解析时返回 `None`。
///
/// 只要**任一**字段解析成功就返回 Some —— 部分机场只给 total 不给 upload/download。
pub fn parse_header(raw: Option<&str>) -> Option<ParsedUserInfo> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }

    let mut out = ParsedUserInfo::default();

    for pair in raw.split(&[';', ','][..]) {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        let val = v.trim().trim_matches('"');

        // 允许值里带单位后缀（少数机场写 "100GB"）——只取前导数字部分
        let num = parse_leading_i64(val);

        match key.as_str() {
            "upload" => out.upload = num,
            "download" => out.download = num,
            "total" => out.total = num,
            "expire" => out.expire_at = num.map(normalize_expire),
            _ => {}
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 取字符串前导数字（支持负数，忽略后续单位）
fn parse_leading_i64(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let mut end = 0;
    let bytes = t.as_bytes();
    let mut start = 0;
    if bytes[0] == b'-' {
        start = 1;
    }
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if b.is_ascii_digit() {
            end = i + 1;
        } else {
            break;
        }
    }
    if end <= start {
        return None;
    }
    t[..end].parse::<i64>().ok()
}

/// 归一化到期时间：毫秒 → 秒。
///
/// 判别依据是量级：Unix 秒在 2001–2033 年约 `1e9`–`2e9`；
/// 毫秒会是 `1e12` 量级。用 1e11 作分界（约公元 5138 年，秒级不会到这）。
pub fn normalize_expire(v: i64) -> i64 {
    const MILLIS_THRESHOLD: i64 = 100_000_000_000; // 1e11
    if v > MILLIS_THRESHOLD {
        v / 1000
    } else {
        v
    }
}

/// 组装成 `UserInfo`（带抓取时间与来源）
pub fn to_user_info(p: ParsedUserInfo, source: UserInfoSource) -> UserInfo {
    UserInfo {
        upload: p.upload,
        download: p.download,
        total: p.total,
        expire_at: p.expire_at,
        fetched_at: now_secs(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_normal_header() {
        let p = parse_header(Some(
            "upload=1073741824; download=2147483648; total=107374182400; expire=1798761600",
        ))
        .expect("应能解析");
        assert_eq!(p.upload, Some(1_073_741_824));
        assert_eq!(p.download, Some(2_147_483_648));
        assert_eq!(p.total, Some(107_374_182_400));
        assert_eq!(p.expire_at, Some(1_798_761_600));
    }

    #[test]
    fn parses_with_spaces_and_any_order() {
        let p = parse_header(Some("  expire = 1798761600 ;total=100;  upload=5;download=6 "))
            .unwrap();
        assert_eq!(p.upload, Some(5));
        assert_eq!(p.download, Some(6));
        assert_eq!(p.total, Some(100));
        assert_eq!(p.expire_at, Some(1_798_761_600));
    }

    #[test]
    fn total_zero_means_unlimited() {
        let p = parse_header(Some("upload=10; download=20; total=0; expire=0")).unwrap();
        assert_eq!(p.total, Some(0), "total=0 原值保留，由上层判为不限量");
        assert_eq!(p.expire_at, Some(0), "expire=0 表示无限期");
    }

    #[test]
    fn missing_header_returns_none() {
        assert_eq!(parse_header(None), None);
        assert_eq!(parse_header(Some("")), None);
        assert_eq!(parse_header(Some("   ")), None);
    }

    #[test]
    fn unparseable_header_returns_none() {
        assert_eq!(parse_header(Some("hello world")), None);
        assert_eq!(parse_header(Some(";;;")), None);
        assert_eq!(parse_header(Some("foo=bar")), None);
    }

    #[test]
    fn partial_fields_are_kept() {
        // 部分机场只给 total
        let p = parse_header(Some("total=1000")).unwrap();
        assert_eq!(p.total, Some(1000));
        assert_eq!(p.upload, None);
        assert_eq!(p.download, None);
    }

    #[test]
    fn only_one_of_upload_or_download() {
        let p = parse_header(Some("upload=100;total=1000")).unwrap();
        assert_eq!(p.upload, Some(100));
        assert_eq!(p.download, None);
        // 上层 used() 会因 download 缺失返回 None —— 这是刻意的保守行为
        let ui = to_user_info(p, UserInfoSource::Header);
        assert_eq!(ui.used(), None);
    }

    #[test]
    fn expire_millis_normalized_to_seconds() {
        // 1798761600000 ms == 1798761600 s
        assert_eq!(normalize_expire(1_798_761_600_000), 1_798_761_600);
        // 秒级原样保留
        assert_eq!(normalize_expire(1_798_761_600), 1_798_761_600);
        assert_eq!(normalize_expire(0), 0);
    }

    #[test]
    fn parses_expire_in_millis_from_header() {
        let p = parse_header(Some("expire=1798761600000")).unwrap();
        assert_eq!(p.expire_at, Some(1_798_761_600), "毫秒必须转秒");
    }

    #[test]
    fn handles_unit_suffix_gracefully() {
        // 少数机场写 "100GB"，取前导数字
        let p = parse_header(Some("total=100GB; upload=5MB")).unwrap();
        assert_eq!(p.total, Some(100), "取前导数字，不因为单位而整条失败");
        assert_eq!(p.upload, Some(5));
    }

    #[test]
    fn handles_quoted_values() {
        let p = parse_header(Some(r#"upload="123"; download="456"; total="789""#)).unwrap();
        assert_eq!(p.upload, Some(123));
        assert_eq!(p.download, Some(456));
        assert_eq!(p.total, Some(789));
    }

    #[test]
    fn keys_are_case_insensitive() {
        let p = parse_header(Some("Upload=1; DOWNLOAD=2; Total=3; EXPIRE=4")).unwrap();
        assert_eq!(p.upload, Some(1));
        assert_eq!(p.download, Some(2));
        assert_eq!(p.total, Some(3));
        assert_eq!(p.expire_at, Some(4));
    }

    #[test]
    fn to_user_info_sets_source_and_time() {
        let p = ParsedUserInfo {
            upload: Some(1),
            ..Default::default()
        };
        let ui = to_user_info(p, UserInfoSource::Header);
        assert_eq!(ui.source, UserInfoSource::Header);
        assert!(ui.fetched_at > 0, "必须记录抓取时间");
    }
}
