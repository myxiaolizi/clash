//! 用户自定义规则库的持久化（v3）。
//!
//! 规则是纯本地数据（用户自己加的分流规则），不进订阅、不被订阅覆盖。
//! 落 `%APPDATA%\ClashZhengl\rules.json`，与 settings.json 同级，
//! 便于用户整体备份。

use std::sync::RwLock;

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{RuleKind, RulePolicy, RuleSet, UserRule};
use crate::system::paths::{read_json_recover, write_json, Paths};

pub struct RulesStore {
    path: std::path::PathBuf,
    inner: RwLock<RuleSet>,
}

impl RulesStore {
    pub fn load(paths: &Paths) -> AppResult<Self> {
        let path = paths.rules_file();
        let set: RuleSet = read_json_recover(&path)?.unwrap_or_default();
        let store = Self {
            path,
            inner: RwLock::new(set),
        };
        // 首次运行写一份空规则库出来，方便用户知道文件在哪、格式长什么样
        if !store.path.exists() {
            if let Err(e) = store.save() {
                tracing::warn!(err = %e, "首次写入规则库失败，继续使用内存默认值");
            }
        }
        Ok(store)
    }

    pub fn list(&self) -> Vec<UserRule> {
        let mut v = self
            .inner
            .read()
            .map(|s| s.rules.clone())
            .unwrap_or_default();
        v.sort_by_key(|r| r.sort_order);
        v
    }

    /// 取规则快照（生成配置用）
    pub fn snapshot(&self) -> RuleSet {
        self.inner
            .read()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// 新增一条规则（追加到末尾）
    pub fn add(
        &self,
        kind: RuleKind,
        value: &str,
        policy: RulePolicy,
        enabled: bool,
    ) -> AppResult<UserRule> {
        let value = validate_value(kind, value)?;
        let mut set = self.write_guard()?;

        // 同类型 + 同值重复添加视为用户误操作，直接拒绝（避免规则段膨胀）
        if set
            .rules
            .iter()
            .any(|r| r.kind == kind && r.value.eq_ignore_ascii_case(&value))
        {
            return Err(AppError::msg(
                ErrorCode::InvalidArg,
                format!("规则已存在：{} {}", kind.as_str(), value),
            ));
        }

        let rule = UserRule {
            id: new_rule_id(),
            kind,
            value,
            policy,
            enabled,
            sort_order: set.rules.len() as u32,
        };
        set.rules.push(rule.clone());
        set.reindex();
        self.save_locked(&set)?;
        tracing::info!(id = %rule.id, kind = rule.kind.as_str(), "规则已添加");
        Ok(rule)
    }

    /// 更新一条规则（未给出的字段保持原值）
    pub fn update(
        &self,
        id: &str,
        kind: Option<RuleKind>,
        value: Option<String>,
        policy: Option<RulePolicy>,
        enabled: Option<bool>,
    ) -> AppResult<UserRule> {
        let mut set = self.write_guard()?;
        let idx = set
            .rules
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;

        let (new_kind, new_value) = {
            let cur = &set.rules[idx];
            let k = kind.unwrap_or(cur.kind);
            let v = match &value {
                Some(v) => validate_value(k, v)?,
                None => validate_value(k, &cur.value)?,
            };
            (k, v)
        };

        // 改完后不得与其它规则重复
        if set.rules.iter().enumerate().any(|(i, r)| {
            i != idx && r.kind == new_kind && r.value.eq_ignore_ascii_case(&new_value)
        }) {
            return Err(AppError::msg(
                ErrorCode::InvalidArg,
                format!("规则已存在：{} {}", new_kind.as_str(), new_value),
            ));
        }

        {
            let r = &mut set.rules[idx];
            r.kind = new_kind;
            r.value = new_value;
            if let Some(p) = policy {
                r.policy = p;
            }
            if let Some(e) = enabled {
                r.enabled = e;
            }
        }
        let out = set.rules[idx].clone();
        self.save_locked(&set)?;
        Ok(out)
    }

    pub fn remove(&self, id: &str) -> AppResult<()> {
        let mut set = self.write_guard()?;
        let before = set.rules.len();
        set.rules.retain(|r| r.id != id);
        if set.rules.len() == before {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        set.reindex();
        self.save_locked(&set)?;
        tracing::info!(id, "规则已删除");
        Ok(())
    }

    /// 按给定 id 顺序重排（前端拖拽/上下移动）。
    ///
    /// 未出现在 `ids` 里的规则追加到末尾，保证不会因前端漏传而丢规则。
    pub fn reorder(&self, ids: &[String]) -> AppResult<Vec<UserRule>> {
        let mut set = self.write_guard()?;

        let mut ordered: Vec<UserRule> = Vec::with_capacity(set.rules.len());
        for id in ids {
            if let Some(pos) = set.rules.iter().position(|r| &r.id == id) {
                ordered.push(set.rules.remove(pos));
            }
        }
        // 剩余未提及的按原顺序追加
        ordered.append(&mut set.rules);
        set.rules = ordered;
        set.reindex();

        let out = set.rules.clone();
        self.save_locked(&set)?;
        Ok(out)
    }

    fn write_guard(&self) -> AppResult<std::sync::RwLockWriteGuard<'_, RuleSet>> {
        self.inner
            .write()
            .map_err(|_| AppError::msg(ErrorCode::Internal, "规则库锁中毒"))
    }

    fn save(&self) -> AppResult<()> {
        let snap = self.snapshot();
        write_json(&self.path, &snap)
    }

    fn save_locked(&self, set: &RuleSet) -> AppResult<()> {
        write_json(&self.path, set)
    }
}

/// 校验并归一化规则的 value。
///
/// 关键安全点：value 会被直接拼进 `rules` 段的 YAML 行，
/// **绝不能含逗号或换行** —— 一个逗号就能伪造出额外的规则字段，
/// 换行能注入任意 YAML 节点（比如塞一个 `proxy-groups` 覆盖我们的组）。
fn validate_value(kind: RuleKind, value: &str) -> AppResult<String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(AppError::msg(ErrorCode::InvalidArg, "规则内容不能为空"));
    }
    if v.contains(',') || v.contains('\n') || v.contains('\r') {
        return Err(AppError::msg(
            ErrorCode::InvalidArg,
            "规则内容不得包含逗号或换行",
        ));
    }
    if v.chars().any(char::is_whitespace) {
        return Err(AppError::msg(
            ErrorCode::InvalidArg,
            "规则内容不得包含空白字符",
        ));
    }
    if v.len() > 256 {
        return Err(AppError::msg(ErrorCode::InvalidArg, "规则内容过长"));
    }

    match kind {
        RuleKind::Ip => {
            // 只接受 IPv4/IPv6 地址或 CIDR，挡掉误填的域名
            if v.parse::<std::net::IpAddr>().is_ok() {
                return Ok(v.to_string());
            }
            // 前缀长度上限取决于地址族：v4 是 32，v6 是 128。
            // 一律按 128 放行会让 `10.0.0.0/99` 这种明显写错的值通过，
            // 内核加载时才报错，用户很难定位。
            let ok = v
                .split_once('/')
                .map(|(a, b)| match a.parse::<std::net::IpAddr>() {
                    Ok(std::net::IpAddr::V4(_)) => b
                        .parse::<u8>()
                        .map(|n| n <= 32)
                        .unwrap_or(false),
                    Ok(std::net::IpAddr::V6(_)) => b
                        .parse::<u8>()
                        .map(|n| n <= 128)
                        .unwrap_or(false),
                    Err(_) => false,
                })
                .unwrap_or(false);
            if !ok {
                return Err(AppError::msg(
                    ErrorCode::InvalidArg,
                    "IP 规则必须是合法地址或 CIDR",
                ));
            }
            Ok(v.to_string())
        }
        RuleKind::Domain | RuleKind::DomainSuffix => {
            if !v.contains('.') || v.contains('/') || v.contains(' ') {
                return Err(AppError::msg(
                    ErrorCode::InvalidArg,
                    "域名规则格式不合法",
                ));
            }
            Ok(v.to_ascii_lowercase())
        }
        RuleKind::DomainKeyword | RuleKind::Process => Ok(v.to_string()),
    }
}

/// 生成规则 id：`r` + 10 位 base32（稳定、可读、不用 uuid 依赖）
fn new_rule_id() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let s: String = (0..10)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();
    format!("r{s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, RulesStore) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        std::fs::create_dir_all(&paths.roaming).unwrap();
        let store = RulesStore::load(&paths).unwrap();
        (dir, store)
    }

    #[test]
    fn starts_empty_and_writes_file() {
        let (_d, store) = setup();
        assert!(store.list().is_empty());
        // 首次加载应把空规则库写出来
        assert!(store.path.exists(), "规则库文件应被创建");
    }

    #[test]
    fn add_list_update_remove() {
        let (_d, store) = setup();
        let r = store
            .add(RuleKind::DomainSuffix, "example.com", RulePolicy::Proxy, true)
            .unwrap();
        assert_eq!(r.value, "example.com");
        assert_eq!(store.list().len(), 1);

        // 改策略与启用状态
        let r2 = store
            .update(&r.id, None, None, Some(RulePolicy::Direct), Some(false))
            .unwrap();
        assert_eq!(r2.policy, RulePolicy::Direct);
        assert!(!r2.enabled);
        assert_eq!(store.list()[0].value, "example.com", "值不应被改动");

        store.remove(&r.id).unwrap();
        assert!(store.list().is_empty());
        assert_eq!(
            store.remove(&r.id).unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn rejects_duplicate_rule() {
        let (_d, store) = setup();
        store
            .add(RuleKind::Domain, "a.com", RulePolicy::Proxy, true)
            .unwrap();
        // 大小写不同也算重复
        let e = store
            .add(RuleKind::Domain, "A.COM", RulePolicy::Proxy, true)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidArg);
    }

    #[test]
    fn rejects_injection_in_value() {
        let (_d, store) = setup();
        // 逗号能伪造规则字段
        assert!(store
            .add(RuleKind::DomainKeyword, "a,DIRECT", RulePolicy::Proxy, true)
            .is_err());
        // 换行能注入 YAML 节点
        assert!(store
            .add(RuleKind::DomainKeyword, "a\nproxy-groups:", RulePolicy::Proxy, true)
            .is_err());
        // 空白会破坏规则行
        assert!(store
            .add(RuleKind::DomainKeyword, "a b", RulePolicy::Proxy, true)
            .is_err());
    }

    #[test]
    fn ip_rule_validation() {
        let (_d, store) = setup();
        assert!(store
            .add(RuleKind::Ip, "1.2.3.4", RulePolicy::Direct, true)
            .is_ok());
        assert!(store
            .add(RuleKind::Ip, "10.0.0.0/8", RulePolicy::Direct, true)
            .is_ok());
        assert!(store
            .add(RuleKind::Ip, "not-an-ip", RulePolicy::Direct, true)
            .is_err());
        assert!(store
            .add(RuleKind::Ip, "10.0.0.0/99", RulePolicy::Direct, true)
            .is_err(), "前缀长度必须合法");
        // v4 上限 32、v6 上限 128，不能混用
        assert!(store
            .add(RuleKind::Ip, "10.0.0.0/33", RulePolicy::Direct, true)
            .is_err(), "IPv4 前缀不得超 32");
        assert!(store
            .add(RuleKind::Ip, "2001:db8::/64", RulePolicy::Direct, true)
            .is_ok(), "IPv6 /64 合法");
        assert!(store
            .add(RuleKind::Ip, "2001:db8::/129", RulePolicy::Direct, true)
            .is_err(), "IPv6 前缀不得超 128");
    }

    #[test]
    fn domain_rule_lowercased() {
        let (_d, store) = setup();
        let r = store
            .add(RuleKind::Domain, "Example.COM", RulePolicy::Proxy, true)
            .unwrap();
        assert_eq!(r.value, "example.com");
    }

    #[test]
    fn reorder_moves_to_front() {
        let (_d, store) = setup();
        let a = store
            .add(RuleKind::Domain, "a.com", RulePolicy::Proxy, true)
            .unwrap();
        let b = store
            .add(RuleKind::Domain, "b.com", RulePolicy::Proxy, true)
            .unwrap();
        let c = store
            .add(RuleKind::Domain, "c.com", RulePolicy::Proxy, true)
            .unwrap();

        let out = store
            .reorder(&[c.id.clone(), a.id.clone(), b.id.clone()])
            .unwrap();
        let ids: Vec<_> = out.iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids, vec![c.id, a.id, b.id]);
        // sort_order 应被重排成 0,1,2
        assert_eq!(
            out.iter().map(|r| r.sort_order).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn reorder_keeps_unmentioned_rules() {
        let (_d, store) = setup();
        let a = store
            .add(RuleKind::Domain, "a.com", RulePolicy::Proxy, true)
            .unwrap();
        let b = store
            .add(RuleKind::Domain, "b.com", RulePolicy::Proxy, true)
            .unwrap();

        // 只传 b：a 不能被丢掉
        let out = store.reorder(&[b.id.clone()]).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, b.id);
        assert_eq!(out[1].id, a.id);
    }

    #[test]
    fn persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        std::fs::create_dir_all(&paths.roaming).unwrap();

        {
            let store = RulesStore::load(&paths).unwrap();
            store
                .add(RuleKind::DomainSuffix, "x.com", RulePolicy::Block, true)
                .unwrap();
        }
        let store2 = RulesStore::load(&paths).unwrap();
        assert_eq!(store2.list().len(), 1);
        assert_eq!(store2.list()[0].policy, RulePolicy::Block);
    }

    #[test]
    fn renders_mihomo_lines_in_order() {
        let (_d, store) = setup();
        store
            .add(RuleKind::DomainSuffix, "google.com", RulePolicy::Proxy, true)
            .unwrap();
        store
            .add(RuleKind::DomainKeyword, "ads", RulePolicy::Block, true)
            .unwrap();
        let off = store
            .add(RuleKind::Domain, "off.com", RulePolicy::Direct, false)
            .unwrap();
        assert!(!off.enabled);

        let lines = store.snapshot().to_mihomo_lines("Proxy");
        assert_eq!(
            lines,
            vec!["DOMAIN-SUFFIX,google.com,Proxy", "DOMAIN-KEYWORD,ads,REJECT"],
            "停用规则不得出现"
        );
    }

    #[test]
    fn ip_rule_gets_no_resolve() {
        let (_d, store) = setup();
        store
            .add(RuleKind::Ip, "10.0.0.0/8", RulePolicy::Direct, true)
            .unwrap();
        let lines = store.snapshot().to_mihomo_lines("Proxy");
        assert_eq!(lines[0], "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve");
    }

    #[test]
    fn corrupt_file_falls_back_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        std::fs::create_dir_all(&paths.roaming).unwrap();
        std::fs::write(paths.rules_file(), b"{bad json").unwrap();

        let store = RulesStore::load(&paths).unwrap();
        assert!(store.list().is_empty(), "损坏时回退空库而非崩溃");
        // 坏文件应被备份
        let has_backup = std::fs::read_dir(&paths.roaming)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("corrupt-"));
        assert!(has_backup);
    }
}
