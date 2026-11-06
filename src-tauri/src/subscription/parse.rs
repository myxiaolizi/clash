//! 订阅 YAML 解析与合法性校验（P2）。
//!
//! 目标：从机场返回的 mihomo/clash 配置里提取
//! - 节点列表（名称、类型）
//! - 策略组名
//! - 可选的其他字段（用于合并时保留）
//!
//! 校验要点（附录 A / 04 章）：
//! - `proxies` 必须是非空数组，否则 `E_PARSE_EMPTY`
//! - 每个节点必须有 `name` 与 `type`
//! - 节点名重复是常见脏数据，需去重（保留首个）

use serde::{Deserialize, Serialize};

use crate::domain::errors::{AppError, AppResult, ErrorCode};

/// 单个节点（只保留我们需要的字段）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParsedProxy {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// 原始 YAML 节点（合并时原样透传，避免丢字段）
    #[serde(skip)]
    pub raw: Option<serde_yaml_ng::Value>,
}

/// 解析结果
#[derive(Debug, Clone, Default)]
pub struct ParsedSubscription {
    pub proxies: Vec<ParsedProxy>,
    pub group_names: Vec<String>,
    /// 原始 proxy-groups（合并时参考）
    pub raw_groups: Vec<serde_yaml_ng::Value>,
    /// 节点类型分布（诊断用）
    pub type_counts: Vec<(String, usize)>,
    /// 被去重丢弃的节点名
    pub duplicates: Vec<String>,
    /// 因缺字段被跳过的条目
    pub skipped: usize,
}

impl ParsedSubscription {
    pub fn node_count(&self) -> usize {
        self.proxies.len()
    }
}

/// 解析订阅 YAML 文本
pub fn parse_subscription(text: &str) -> AppResult<ParsedSubscription> {
    if text.trim().is_empty() {
        return Err(AppError::msg(ErrorCode::SubEmpty, "订阅内容为空"));
    }

    // HTML 必须**在解析前**拦掉。
    // 原因：`<html>...</html>` 在 YAML 里是一个合法标量文档，serde 会解析成功，
    // 之后只会报"根节点不是对象"——用户看到这个提示完全不知道该去查登录态。
    let sniff = text.trim_start();
    if sniff.starts_with('<') || sniff.to_ascii_lowercase().contains("<html") {
        return Err(AppError::msg(
            ErrorCode::ParseYaml,
            "返回内容是 HTML（可能是登录页或错误页），不是 Clash 配置",
        ));
    }

    // 部分机场返回的是 base64 包装的 YAML（旧版 clash 订阅格式）
    let decoded = maybe_base64_decode(text);
    let body = decoded.as_deref().unwrap_or(text);

    let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(body).map_err(|e| {
        AppError::msg(
            ErrorCode::ParseYaml,
            format!("YAML 语法错误：{}", crate::domain::errors::truncate(&e.to_string(), 200)),
        )
    })?;

    let Some(map) = doc.as_mapping() else {
        return Err(AppError::msg(
            ErrorCode::ParseYaml,
            "配置根节点不是对象（期望 YAML 映射，实际是标量或数组）",
        ));
    };

    // ---- proxies ----
    let mut proxies: Vec<ParsedProxy> = Vec::new();
    let mut duplicates = Vec::new();
    let mut skipped = 0usize;
    let mut seen = std::collections::HashSet::new();

    if let Some(list) = map.get(serde_yaml_ng::Value::String("proxies".into())) {
        if let Some(seq) = list.as_sequence() {
            for item in seq {
                let Some(m) = item.as_mapping() else {
                    skipped += 1;
                    continue;
                };
                let name = m
                    .get(serde_yaml_ng::Value::String("name".into()))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let kind = m
                    .get(serde_yaml_ng::Value::String("type".into()))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let (Some(name), Some(kind)) = (name, kind) else {
                    skipped += 1;
                    continue;
                };

                if !seen.insert(name.clone()) {
                    duplicates.push(name);
                    continue;
                }

                proxies.push(ParsedProxy {
                    name,
                    kind,
                    raw: Some(item.clone()),
                });
            }
        }
    }

    if proxies.is_empty() {
        return Err(AppError::msg(
            ErrorCode::ParseEmpty,
            "配置中没有可用节点（proxies 为空或全部缺少 name/type）",
        ));
    }

    // ---- proxy-groups ----
    let mut group_names = Vec::new();
    let mut raw_groups = Vec::new();
    if let Some(list) = map.get(serde_yaml_ng::Value::String("proxy-groups".into())) {
        if let Some(seq) = list.as_sequence() {
            for item in seq {
                raw_groups.push(item.clone());
                if let Some(name) = item
                    .as_mapping()
                    .and_then(|m| m.get(serde_yaml_ng::Value::String("name".into())))
                    .and_then(|v| v.as_str())
                {
                    group_names.push(name.to_string());
                }
            }
        }
    }

    // ---- 类型统计 ----
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in &proxies {
        *counts.entry(p.kind.clone()).or_insert(0) += 1;
    }
    let mut type_counts: Vec<(String, usize)> = counts.into_iter().collect();
    type_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    tracing::info!(
        nodes = proxies.len(),
        groups = group_names.len(),
        duplicates = duplicates.len(),
        skipped,
        "订阅解析完成"
    );

    Ok(ParsedSubscription {
        proxies,
        group_names,
        raw_groups,
        type_counts,
        duplicates,
        skipped,
    })
}

/// 尝试 base64 解码（旧版订阅格式）。
///
/// 只在「解码成功且结果看起来像 YAML」时才采纳，否则返回 None 走原文路径。
fn maybe_base64_decode(text: &str) -> Option<String> {
    use base64::Engine;
    let t = text.trim();
    // 含 YAML 特征则不当作 base64
    if t.contains(':') || t.contains('\n') && t.contains("proxies") {
        return None;
    }
    if t.len() < 16 || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=') {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD.decode(t).ok()?;
    let s = String::from_utf8(decoded).ok()?;
    if s.contains(':') && (s.contains("proxies") || s.contains("proxy-groups") || s.contains("port")) {
        Some(s)
    } else {
        None
    }
}

/// 快速判定响应体是否像 Clash 配置（用于提前给出更准的错误）
pub fn looks_like_clash_config(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with('<') {
        return false;
    }
    t.contains("proxies:") || t.contains("proxy-groups:") || maybe_base64_decode(text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
port: 7890
socks-port: 7891
allow-lan: false
mode: rule
proxies:
  - name: "香港 01"
    type: ss
    server: a.example.com
    port: 8388
    cipher: aes-256-gcm
    password: "test"
  - name: "日本 01"
    type: vmess
    server: b.example.com
    port: 443
    uuid: abc
  - name: "香港 01"
    type: ss
    server: dup.example.com
    port: 8388
  - name: ""
    type: ss
proxy-groups:
  - name: Proxy
    type: select
    proxies: ["香港 01", "日本 01"]
  - name: Auto
    type: url-test
    proxies: ["香港 01"]
rules:
  - MATCH,Proxy
"#;

    #[test]
    fn parses_sample() {
        let p = parse_subscription(SAMPLE).unwrap();
        assert_eq!(p.node_count(), 2, "重复名与空名应被剔除");
        assert_eq!(p.proxies[0].name, "香港 01");
        assert_eq!(p.proxies[0].kind, "ss");
        assert_eq!(p.group_names, vec!["Proxy", "Auto"]);
        assert_eq!(p.duplicates, vec!["香港 01"]);
        assert_eq!(p.skipped, 1, "空 name 条目被跳过");
    }

    #[test]
    fn preserves_raw_proxy_for_merge() {
        let p = parse_subscription(SAMPLE).unwrap();
        let raw = p.proxies[0].raw.as_ref().expect("必须保留原始节点");
        let m = raw.as_mapping().unwrap();
        // 合并时需要原样透传所有字段（cipher/password 等不能丢）
        assert!(m.contains_key(serde_yaml_ng::Value::String("password".into())));
        assert!(m.contains_key(serde_yaml_ng::Value::String("cipher".into())));
    }

    #[test]
    fn type_counts_sorted_desc() {
        let p = parse_subscription(SAMPLE).unwrap();
        assert_eq!(p.type_counts[0], ("ss".to_string(), 1));
        assert_eq!(p.type_counts.len(), 2);
    }

    #[test]
    fn empty_text_is_sub_empty() {
        let e = parse_subscription("").unwrap_err();
        assert_eq!(e.code, ErrorCode::SubEmpty);
        assert_eq!(parse_subscription("   ").unwrap_err().code, ErrorCode::SubEmpty);
    }

    #[test]
    fn html_body_gives_readable_error() {
        let e = parse_subscription("<html><body>Login required</body></html>").unwrap_err();
        assert_eq!(e.code, ErrorCode::ParseYaml);
        assert!(e.message.contains("HTML"), "应提示返回的是 HTML: {}", e.message);
    }

    #[test]
    fn empty_proxies_is_parse_empty() {
        let e = parse_subscription("proxies: []\n").unwrap_err();
        assert_eq!(e.code, ErrorCode::ParseEmpty);
    }

    #[test]
    fn missing_proxies_key_is_parse_empty() {
        let e = parse_subscription("port: 7890\n").unwrap_err();
        assert_eq!(e.code, ErrorCode::ParseEmpty);
    }

    #[test]
    fn invalid_yaml_is_parse_yaml_error() {
        let e = parse_subscription("proxies: [\n  - {name: a").unwrap_err();
        assert_eq!(e.code, ErrorCode::ParseYaml);
    }

    #[test]
    fn root_not_mapping_errors() {
        let e = parse_subscription("- a\n- b\n").unwrap_err();
        assert_eq!(e.code, ErrorCode::ParseYaml);
        assert!(e.message.contains("根节点"));
    }

    #[test]
    fn base64_wrapped_config_is_decoded() {
        use base64::Engine;
        let inner = "proxies:\n  - name: n1\n    type: ss\n    server: a.com\n    port: 1\n";
        let b64 = base64::engine::general_purpose::STANDARD.encode(inner);
        let p = parse_subscription(&b64).unwrap();
        assert_eq!(p.node_count(), 1);
    }

    #[test]
    fn base64_looking_but_not_config_falls_through() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode("random text without yaml");
        // 解码后不是配置 → 走原文 YAML 解析路径 → 报错（而不是误判成配置）
        let e = parse_subscription(&b64).unwrap_err();
        assert_eq!(e.code, ErrorCode::ParseYaml);
    }

    #[test]
    fn looks_like_clash_config_detection() {
        assert!(looks_like_clash_config(SAMPLE));
        assert!(!looks_like_clash_config("<html></html>"));
        assert!(!looks_like_clash_config("just some text"));
    }

    #[test]
    fn node_without_type_skipped() {
        let yaml = "proxies:\n  - name: a\n  - name: b\n    type: ss\n";
        let p = parse_subscription(yaml).unwrap();
        assert_eq!(p.node_count(), 1);
        assert_eq!(p.skipped, 1);
    }
}
