//! 多订阅合并 → `runtime/config.yaml`（P2 核心，最易出致命错的地方）。
//!
//! ## 三条硬约束（附录 A.6.4，违反会导致功能静默失效）
//!
//! 1. **所有 proxy-groups 必须 `type: select`**
//!    `url-test` / `fallback` / `load-balance` 会自己挑节点，与我们的自动切换
//!    形成两个决策者互相抢，用户会看到"刚选好的节点几秒后被改回去"。
//!    `assert_no_auto_groups()` 在写盘前强制拦截。
//!
//! 2. **不写 `profile-update-interval`**
//!    写了之后内核会自己更新订阅，与我们的调度器重复拉取，浪费流量且互相覆盖。
//!
//! 3. **`find-process-mode: strict`**
//!    设成 `off` 会让 `helper.FindProcess=nil`，机场的 PROCESS-NAME 规则永久静默失效
//!    （不报错，就是不走规则，极难排查）。
//!
//! ## DNS 策略
//! SystemProxy 档 `dns.enable: false`：交给系统解析，避免与系统 DNS 抢
//! （TUN 档才需要内核接管 DNS）。使用 `store-fake-ip: false` 防止假 IP 污染缓存。

use std::collections::BTreeSet;

use serde_yaml_ng::{Mapping, Value};

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::MihomoMode;

/// 主组名（固定，前端与切换逻辑都依赖它）
pub const MAIN_GROUP_DEFAULT: &str = "Proxy";

/// 生成配置所需的输入
pub struct GenerateInput<'a> {
    /// 各订阅解析后的节点（按订阅顺序）
    pub subscriptions: Vec<SubNodes<'a>>,
    pub mixed_port: u16,
    pub api_port: u16,
    pub secret: String,
    /// 主组名
    pub main_group: String,
    pub log_level: &'a str,
    /// 是否有 TUN 需求（决定 dns.enable 与 tun 段）
    pub tun_enabled: bool,
    /// 内核分流模式（rule / global / direct）。
    ///
    /// 早期这里是硬编码 `mode: rule`，用户切「全局」只改了个前端开关，
    /// 配置里还是 rule，于是"全局模式不生效"——这是必须写进配置的字段。
    pub mihomo_mode: MihomoMode,
    /// 用户自定义规则（已渲染成 `TYPE,value,target` 行，顺序即优先级）。
    ///
    /// 插在 rules 段**最前**：用户的显式意图优先级高于我们兜底的 MATCH。
    pub user_rules: Vec<String>,
}

pub struct SubNodes<'a> {
    /// 订阅显示名（用于节点前缀）
    pub sub_name: &'a str,
    /// 节点是否加订阅名前缀（重名时必需）
    pub prefix: bool,
    pub proxies: Vec<(String, Value)>,
}

/// 生成结果
#[derive(Debug)]
pub struct Generated {
    pub yaml: String,
    /// 最终写入的节点名列表（顺序即分组顺序）
    pub node_names: Vec<String>,
    pub group_name: String,
}

/// 生成 mihomo 配置
pub fn generate(input: &GenerateInput<'_>) -> AppResult<Generated> {
    // ---- 1. 汇总节点，处理重名 ----
    let mut proxies_seq: Vec<Value> = Vec::new();
    let mut node_names: Vec<String> = Vec::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut renames: Vec<(String, String)> = Vec::new();

    for sub in &input.subscriptions {
        for (name, raw) in &sub.proxies {
            // 跨订阅重名是常态（不同机场都有 "香港 01"），必须消歧
            let mut candidate = if sub.prefix {
                format!("[{}] {}", sub.sub_name, name)
            } else {
                name.clone()
            };

            if used.contains(&candidate) {
                // 加序号消歧，保证稳定（后缀递增）
                let base = candidate.clone();
                let mut n = 2;
                loop {
                    candidate = format!("{base} #{n}");
                    if !used.contains(&candidate) {
                        break;
                    }
                    n += 1;
                    if n > 999 {
                        return Err(AppError::msg(
                            ErrorCode::ConfigInvalid,
                            format!("节点名重复过多，无法消歧：{base}"),
                        ));
                    }
                }
                renames.push((base, candidate.clone()));
            }

            used.insert(candidate.clone());

            // 改写 raw 里的 name 字段，保持与分组引用一致
            let mut m = raw.as_mapping().cloned().ok_or_else(|| {
                AppError::msg(ErrorCode::ConfigInvalid, "节点不是对象")
            })?;
            m.insert(
                Value::String("name".into()),
                Value::String(candidate.clone()),
            );
            proxies_seq.push(Value::Mapping(m));
            node_names.push(candidate);
        }
    }

    if node_names.is_empty() {
        return Err(AppError::msg(
            ErrorCode::NoSubscription,
            "没有可用节点，请先添加订阅",
        ));
    }

    if !renames.is_empty() {
        tracing::info!(count = renames.len(), "节点重名已自动消歧");
        for (from, to) in renames.iter().take(5) {
            tracing::debug!(from = %from, to = %to, "重命名");
        }
    }

    // ---- 2. 主组：**只有 select** ----
    let mut group = Mapping::new();
    group.insert(
        Value::String("name".into()),
        Value::String(input.main_group.clone()),
    );
    // 硬编码 select，绝不允许其它类型（见模块文档约束 1）
    group.insert(
        Value::String("type".into()),
        Value::String("select".into()),
    );
    group.insert(
        Value::String("proxies".into()),
        Value::Sequence(node_names.iter().map(|n| Value::String(n.clone())).collect()),
    );

    // ---- 3. 组装根配置 ----
    let mut root = Mapping::new();
    let s = |v: &str| Value::String(v.to_string());
    let b = |v: bool| Value::Bool(v);
    let n = |v: u64| Value::Number(serde_yaml_ng::Number::from(v));

    root.insert(s("mixed-port"), n(input.mixed_port as u64));
    root.insert(s("allow-lan"), b(false));
    root.insert(s("bind-address"), s("127.0.0.1"));
    // 分流模式必须真正写进配置，否则前端切「全局」不生效
    root.insert(s("mode"), s(input.mihomo_mode.as_str()));
    root.insert(s("log-level"), s(input.log_level));
    // 约束 3：strict，不能用 off
    root.insert(s("find-process-mode"), s("strict"));
    root.insert(s("ipv6"), b(true));

    root.insert(s("external-controller"), s(&format!("127.0.0.1:{}", input.api_port)));
    root.insert(s("secret"), s(&input.secret));

    // 约束 2：**不写** profile-update-interval。
    // 只写 store-selected / store-fake-ip 两项。
    let mut profile = Mapping::new();
    profile.insert(s("store-selected"), b(true));
    // 假 IP 缓存会污染 DNS，SystemProxy 档不需要
    profile.insert(s("store-fake-ip"), b(input.tun_enabled));
    root.insert(s("profile"), Value::Mapping(profile));

    // DNS：SystemProxy 档关闭，TUN 档打开
    let mut dns = Mapping::new();
    dns.insert(s("enable"), b(input.tun_enabled));
    if input.tun_enabled {
        dns.insert(s("ipv6"), b(true));
        dns.insert(s("enhanced-mode"), s("fake-ip"));
        dns.insert(s("fake-ip-range"), s("198.18.0.1/16"));
        dns.insert(
            s("nameserver"),
            Value::Sequence(vec![
                s("https://223.5.5.5/dns-query"),
                s("https://1.12.12.12/dns-query"),
            ]),
        );
        dns.insert(
            s("fallback"),
            Value::Sequence(vec![
                s("https://8.8.8.8/dns-query"),
                s("tls://1.1.1.1:853"),
            ]),
        );
    }
    root.insert(s("dns"), Value::Mapping(dns));

    // TUN 段（仅 TUN 档）
    if input.tun_enabled {
        let mut tun = Mapping::new();
        tun.insert(s("enable"), b(true));
        tun.insert(s("stack"), s("mixed"));
        tun.insert(s("auto-route"), b(true));
        tun.insert(s("auto-detect-interface"), b(true));
        tun.insert(s("dns-hijack"), Value::Sequence(vec![s("any:53")]));
        root.insert(s("tun"), Value::Mapping(tun));
    }

    root.insert(s("proxies"), Value::Sequence(proxies_seq));

    let mut groups_seq = vec![Value::Mapping(group)];
    root.insert(s("proxy-groups"), Value::Sequence(groups_seq.drain(..).collect()));

    // 规则：用户规则在前（显式意图优先），最后兜底 MATCH 走主组
    let mut rules: Vec<String> = input.user_rules.clone();
    rules.push(format!("MATCH,{}", input.main_group));
    // 去重保序：用户可能手抖加了两条一样的关键词规则，
    // mihomo 对重复规则不报错但会浪费匹配；顺手清掉更干净。
    let mut seen_rule = std::collections::HashSet::new();
    rules.retain(|r| seen_rule.insert(r.clone()));
    root.insert(
        s("rules"),
        Value::Sequence(rules.iter().map(|r| s(r)).collect()),
    );

    // ---- 4. 序列化 ----
    let yaml = serde_yaml_ng::to_string(&Value::Mapping(root)).map_err(|e| {
        AppError::with_detail(ErrorCode::ConfigInvalid, format!("配置序列化失败: {e}"))
    })?;

    // ---- 5. 写盘前强制断言（最关键的一步）----
    assert_no_auto_groups(&yaml)?;

    Ok(Generated {
        yaml,
        node_names,
        group_name: input.main_group.clone(),
    })
}

/// **强制断言**：配置里不得出现任何自动类型策略组。
///
/// 用「解析后检查」而非「检查我们自己的代码」，因为前者能挡住
/// 未来任何人引入的间接路径（比如保留订阅原始 proxy-groups）。
pub fn assert_no_auto_groups(yaml: &str) -> AppResult<()> {
    let doc: Value = serde_yaml_ng::from_str(yaml).map_err(|e| {
        AppError::with_detail(ErrorCode::ConfigInvalid, format!("生成配置无法回读: {e}"))
    })?;

    let Some(map) = doc.as_mapping() else {
        return Err(AppError::msg(ErrorCode::ConfigInvalid, "配置根节点不是对象"));
    };

    if let Some(groups) = map.get(Value::String("proxy-groups".into())) {
        if let Some(seq) = groups.as_sequence() {
            for (i, g) in seq.iter().enumerate() {
                let Some(m) = g.as_mapping() else {
                    return Err(AppError::msg(
                        ErrorCode::ConfigInvalid,
                        format!("proxy-groups[{i}] 不是对象"),
                    ));
                };
                let kind = m
                    .get(Value::String("type".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let name = m
                    .get(Value::String("name".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("<未命名>");

                // 只允许 select。注意这里比的是 **YAML 值**（小写 select），
                // 与 API 返回的 "Selector" 是两套命名空间，不要混用。
                if kind != "select" {
                    return Err(AppError::msg(
                        ErrorCode::ConfigInvalid,
                        format!(
                            "禁止生成 type={kind} 的自动策略组（组 {name}）：会与自动切换争抢节点控制权"
                        ),
                    ));
                }
            }
        }
    }

    // 同时确保没有 profile-update-interval（约束 2）
    if let Some(profile) = map.get(Value::String("profile".into())) {
        if let Some(pm) = profile.as_mapping() {
            if pm.contains_key(Value::String("profile-update-interval".into())) {
                return Err(AppError::msg(
                    ErrorCode::ConfigInvalid,
                    "禁止写入 profile-update-interval：会与内置调度器重复更新订阅",
                ));
            }
        }
    }

    // find-process-mode 不得为 off
    if let Some(v) = map.get(Value::String("find-process-mode".into())) {
        if v.as_str() == Some("off") {
            return Err(AppError::msg(
                ErrorCode::ConfigInvalid,
                "find-process-mode 不得为 off：会让 PROCESS 类规则永久静默失效",
            ));
        }
    }

    Ok(())
}

/// 校验内核能接受该配置（调 `-t` 测试）。
///
/// 返回 Err 时**不要**覆盖现有配置，保持上一个可用版本。
pub async fn validate_with_core(exe: &std::path::Path, config: &std::path::Path) -> AppResult<()> {
    let out = tokio::process::Command::new(exe)
        .arg("-t")
        .arg("-f")
        .arg(config)
        .output()
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::ConfigInvalid, e))?;

    if out.status.success() {
        return Ok(());
    }

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Err(AppError::msg(
        ErrorCode::ConfigInvalid,
        format!(
            "内核校验配置失败：{}",
            crate::domain::errors::truncate(&crate::util::redact::sanitize(&text), 400)
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(name: &str, kind: &str) -> (String, Value) {
        let mut m = Mapping::new();
        m.insert(Value::String("name".into()), Value::String(name.into()));
        m.insert(Value::String("type".into()), Value::String(kind.into()));
        m.insert(Value::String("server".into()), Value::String("a.example.com".into()));
        m.insert(Value::String("port".into()), Value::Number(serde_yaml_ng::Number::from(8388)));
        (name.to_string(), Value::Mapping(m))
    }

    fn base_input<'a>(subs: Vec<SubNodes<'a>>) -> GenerateInput<'a> {
        GenerateInput {
            subscriptions: subs,
            mixed_port: 10801,
            api_port: 49152,
            secret: "deadbeef".into(),
            main_group: MAIN_GROUP_DEFAULT.to_string(),
            log_level: "info",
            tun_enabled: false,
            mihomo_mode: MihomoMode::Rule,
            user_rules: Vec::new(),
        }
    }

    #[test]
    fn generates_single_select_group() {
        let subs = vec![SubNodes {
            sub_name: "机场A",
            prefix: false,
            proxies: vec![proxy("香港 01", "ss"), proxy("日本 01", "vmess")],
        }];
        let g = generate(&base_input(subs)).unwrap();

        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let m = doc.as_mapping().unwrap();
        let groups = m.get(Value::String("proxy-groups".into())).unwrap().as_sequence().unwrap();
        assert_eq!(groups.len(), 1, "只生成一个主组");
        let g0 = groups[0].as_mapping().unwrap();
        assert_eq!(g0.get(Value::String("name".into())).unwrap().as_str(), Some("Proxy"));
        assert_eq!(
            g0.get(Value::String("type".into())).unwrap().as_str(),
            Some("select"),
            "主组必须是 select"
        );
    }

    #[test]
    fn never_emits_auto_group_types() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        for bad in ["url-test", "fallback", "load-balance", "relay"] {
            assert!(
                !g.yaml.contains(&format!("type: {bad}")),
                "禁止出现 type: {bad}"
            );
        }
    }

    #[test]
    fn assert_rejects_auto_groups() {
        let bad = r#"
proxy-groups:
  - name: Auto
    type: url-test
    proxies: [a]
"#;
        let e = assert_no_auto_groups(bad).unwrap_err();
        assert_eq!(e.code, ErrorCode::ConfigInvalid);
        assert!(e.message.contains("url-test"));
    }

    #[test]
    fn assert_allows_select() {
        let good = r#"
proxy-groups:
  - name: Proxy
    type: select
    proxies: [a]
"#;
        assert!(assert_no_auto_groups(good).is_ok());
    }

    #[test]
    fn assert_rejects_profile_update_interval() {
        let bad = r#"
profile:
  profile-update-interval: 1440
"#;
        let e = assert_no_auto_groups(bad).unwrap_err();
        assert!(e.message.contains("profile-update-interval"));
    }

    #[test]
    fn assert_rejects_find_process_mode_off() {
        let bad = "find-process-mode: off\n";
        let e = assert_no_auto_groups(bad).unwrap_err();
        assert!(e.message.contains("find-process-mode"));
    }

    #[test]
    fn find_process_mode_is_strict() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        assert!(g.yaml.contains("find-process-mode: strict"));
        assert!(!g.yaml.contains("find-process-mode: off"));
    }

    #[test]
    fn no_profile_update_interval_emitted() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        assert!(
            !g.yaml.contains("profile-update-interval"),
            "不得让内核自己更新订阅"
        );
    }

    #[test]
    fn cross_subscription_duplicates_are_disambiguated() {
        let subs = vec![
            SubNodes {
                sub_name: "机场A",
                prefix: false,
                proxies: vec![proxy("香港 01", "ss")],
            },
            SubNodes {
                sub_name: "机场B",
                prefix: false,
                proxies: vec![proxy("香港 01", "ss")],
            },
        ];
        let g = generate(&base_input(subs)).unwrap();
        assert_eq!(g.node_names.len(), 2);
        assert_ne!(g.node_names[0], g.node_names[1], "重名必须消歧");
        assert!(g.node_names.iter().all(|n| n.contains("香港 01")));
    }

    #[test]
    fn prefix_mode_keeps_subscription_identity() {
        let subs = vec![
            SubNodes {
                sub_name: "机场A",
                prefix: true,
                proxies: vec![proxy("香港 01", "ss")],
            },
            SubNodes {
                sub_name: "机场B",
                prefix: true,
                proxies: vec![proxy("香港 01", "ss")],
            },
        ];
        let g = generate(&base_input(subs)).unwrap();
        assert_eq!(g.node_names[0], "[机场A] 香港 01");
        assert_eq!(g.node_names[1], "[机场B] 香港 01");
    }

    #[test]
    fn duplicate_names_case_insensitive_in_builtin_check_only() {
        // 完全相同名字的三次出现应产生 #2 #3
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![
                proxy("same", "ss"),
                proxy("same", "ss"),
                proxy("same", "ss"),
            ],
        }];
        let g = generate(&base_input(subs)).unwrap();
        assert_eq!(g.node_names, vec!["same", "same #2", "same #3"]);
    }

    #[test]
    fn group_references_all_nodes() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss"), proxy("n2", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let m = doc.as_mapping().unwrap();
        let groups = m.get(Value::String("proxy-groups".into())).unwrap().as_sequence().unwrap();
        let members = groups[0]
            .as_mapping()
            .unwrap()
            .get(Value::String("proxies".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].as_str(), Some("n1"));
    }

    #[test]
    fn bind_address_is_loopback() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        assert!(g.yaml.contains("bind-address: 127.0.0.1"), "不得监听外部网卡");
        assert!(g.yaml.contains("allow-lan: false"));
    }

    #[test]
    fn dns_disabled_in_systemproxy_mode() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let dns = doc
            .as_mapping()
            .unwrap()
            .get(Value::String("dns".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(
            dns.get(Value::String("enable".into())).unwrap().as_bool(),
            Some(false),
            "SystemProxy 档不接管 DNS"
        );
    }

    #[test]
    fn tun_mode_enables_dns_and_tun() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let mut input = base_input(subs);
        input.tun_enabled = true;
        let g = generate(&input).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let m = doc.as_mapping().unwrap();
        let dns = m.get(Value::String("dns".into())).unwrap().as_mapping().unwrap();
        assert_eq!(dns.get(Value::String("enable".into())).unwrap().as_bool(), Some(true));
        let tun = m.get(Value::String("tun".into())).unwrap().as_mapping().unwrap();
        assert_eq!(tun.get(Value::String("enable".into())).unwrap().as_bool(), Some(true));
    }

    #[test]
    fn no_nodes_is_error() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![],
        }];
        let e = generate(&base_input(subs)).unwrap_err();
        assert_eq!(e.code, ErrorCode::NoSubscription);
    }

    #[test]
    fn generated_config_reparses_cleanly() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: true,
            proxies: vec![proxy("节点 1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        assert!(doc.as_mapping().is_some());
    }

    #[test]
    fn rules_match_main_group() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        assert!(g.yaml.contains("MATCH,Proxy"));
    }

    // ---- v3：分流模式 ----

    fn one_node_input() -> Vec<SubNodes<'static>> {
        vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }]
    }

    #[test]
    fn mode_rule_written_by_default() {
        let g = generate(&base_input(one_node_input())).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let mode = doc
            .as_mapping()
            .unwrap()
            .get(Value::String("mode".into()))
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(mode, "rule");
    }

    #[test]
    fn mode_global_actually_written() {
        // 回归：早期 mode 硬编码 rule，切「全局」只改了前端开关，配置没变
        let mut input = base_input(one_node_input());
        input.mihomo_mode = MihomoMode::Global;
        let g = generate(&input).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let mode = doc
            .as_mapping()
            .unwrap()
            .get(Value::String("mode".into()))
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(mode, "global", "全局模式必须写进配置");
    }

    #[test]
    fn mode_direct_actually_written() {
        let mut input = base_input(one_node_input());
        input.mihomo_mode = MihomoMode::Direct;
        let g = generate(&input).unwrap();
        assert!(g.yaml.contains("mode: direct"));
    }

    // ---- v3：用户规则 ----

    #[test]
    fn user_rules_come_before_match() {
        let mut input = base_input(one_node_input());
        input.user_rules = vec![
            "DOMAIN-SUFFIX,google.com,Proxy".into(),
            "DOMAIN-KEYWORD,ads,REJECT".into(),
        ];
        let g = generate(&input).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let rules = doc
            .as_mapping()
            .unwrap()
            .get(Value::String("rules".into()))
            .unwrap()
            .as_sequence()
            .unwrap();

        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].as_str(), Some("DOMAIN-SUFFIX,google.com,Proxy"));
        assert_eq!(rules[1].as_str(), Some("DOMAIN-KEYWORD,ads,REJECT"));
        // MATCH 必须是最后一条，否则它上面的规则永远不生效
        assert_eq!(rules[2].as_str(), Some("MATCH,Proxy"));
    }

    #[test]
    fn duplicate_user_rules_deduped() {
        let mut input = base_input(one_node_input());
        input.user_rules = vec![
            "DOMAIN,a.com,DIRECT".into(),
            "DOMAIN,a.com,DIRECT".into(),
        ];
        let g = generate(&input).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&g.yaml).unwrap();
        let rules = doc
            .as_mapping()
            .unwrap()
            .get(Value::String("rules".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        assert_eq!(rules.len(), 2, "重复规则应被去掉，只剩一条 + MATCH");
    }

    #[test]
    fn secret_and_ports_present() {
        let subs = vec![SubNodes {
            sub_name: "A",
            prefix: false,
            proxies: vec![proxy("n1", "ss")],
        }];
        let g = generate(&base_input(subs)).unwrap();
        assert!(g.yaml.contains("secret: deadbeef"));
        assert!(g.yaml.contains("mixed-port: 10801"));
        assert!(g.yaml.contains("external-controller: 127.0.0.1:49152"));
    }
}
