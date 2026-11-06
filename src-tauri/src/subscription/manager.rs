//! 订阅管理编排（P2）：增删改、拉取落盘、合并生成配置。
//!
//! 不直接依赖 UI/内核，靠注入的 `SelectorApi` 与路径工作。

use std::sync::Arc;

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{
    Subscription, SubscriptionMeta, UpdateStatus,
};
use crate::store::crypto;
use crate::store::settings_store::{SettingsStore, SubscriptionStore};
use crate::subscription::fetch::{fetch_subscription, is_due, FetchOutcome};
use crate::subscription::generator::{generate, GenerateInput, Generated, SubNodes};
use crate::subscription::parse::ParsedSubscription;
use crate::system::paths::{atomic_write, Paths};
use crate::util::redact::url_hint;
use crate::util::time::now_secs;

pub struct SubscriptionManager {
    paths: Arc<Paths>,
    subs: Arc<SubscriptionStore>,
    settings: Arc<SettingsStore>,
    /// 串行化「拉取 + 生成配置」，避免并发写 config.yaml
    fetch_lock: tokio::sync::Mutex<()>,
}

impl SubscriptionManager {
    pub fn new(
        paths: Arc<Paths>,
        subs: Arc<SubscriptionStore>,
        settings: Arc<SettingsStore>,
    ) -> Self {
        Self {
            paths,
            subs,
            settings,
            fetch_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn list(&self) -> Vec<Subscription> {
        self.subs.list()
    }

    /// 新增订阅：校验 URL → DPAPI 加密 → 生成 uid → 落盘
    pub fn add(&self, name: &str, url: &str) -> AppResult<Subscription> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::msg(ErrorCode::InvalidArg, "订阅名称不能为空"));
        }
        // URL 合法性先校验，避免存进去一个永远拉不动的订阅
        crate::util::net::validate_subscription_url(url)?;

        let uid = generate_uid();
        let cipher = crypto::protect(url.trim())?;
        let hint = url_hint(url.trim());
        let interval = self.settings.get().sub_default_interval_min;
        let sort_order = self
            .subs
            .list()
            .iter()
            .map(|s| s.sort_order)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        let mut sub = Subscription::new(uid, name.to_string(), cipher, hint, interval);
        sub.sort_order = sort_order;
        self.subs.upsert(sub.clone())?;
        tracing::info!(uid = %sub.uid, name = %sub.name, "订阅已添加");
        Ok(sub)
    }

    pub fn remove(&self, uid: &str) -> AppResult<()> {
        if self.subs.get(uid).is_none() {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        self.subs.remove(uid)?;
        tracing::info!(uid, "订阅已删除");
        Ok(())
    }

    pub fn set_enabled(&self, uid: &str, enabled: bool) -> AppResult<Subscription> {
        let mut sub = self
            .subs
            .get(uid)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        sub.enabled = enabled;
        self.subs.upsert(sub.clone())?;
        Ok(sub)
    }

    pub fn update_interval(&self, uid: &str, interval_min: u32) -> AppResult<Subscription> {
        let mut sub = self
            .subs
            .get(uid)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        // 周期下限 15 分钟（C.6 硬约束）
        sub.update_interval_min = interval_min.max(15);
        self.subs.upsert(sub.clone())?;
        Ok(sub)
    }

    pub fn rename(&self, uid: &str, new_name: &str) -> AppResult<Subscription> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(AppError::msg(ErrorCode::InvalidArg, "订阅名称不能为空"));
        }
        let mut sub = self
            .subs
            .get(uid)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        sub.name = new_name.to_string();
        self.subs.upsert(sub.clone())?;
        Ok(sub)
    }

    /// 解密订阅 URL（仅在需要发请求时调用，绝不出现在日志/前端）
    pub fn decrypt_url(&self, uid: &str) -> AppResult<String> {
        let sub = self
            .subs
            .get(uid)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        crypto::unprotect(&sub.url_cipher)
    }

    /// 判断哪些订阅本次到期（调度器用）
    pub fn due_uids(&self, now: i64) -> Vec<String> {
        self.subs
            .list()
            .into_iter()
            .filter(|s| s.enabled)
            .filter(|s| is_due(s.last_update_at, s.update_interval_min, now))
            .map(|s| s.uid)
            .collect()
    }

    /// 拉取单个订阅并落盘。`force=true` 时忽略 ETag（用户点"立即更新"）。
    pub async fn fetch_one(&self, uid: &str, force: bool) -> AppResult<ParsedSubscription> {
        let sub = self
            .subs
            .get(uid)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let settings = self.settings.get();
        let url = self.decrypt_url(uid)?;
        let meta = self.subs.read_meta(uid)?;
        let cached = self.subs.read_raw_yaml(uid)?;

        let (etag, last_mod) = if force {
            (None, None)
        } else {
            (meta.etag.as_deref(), meta.last_modified.as_deref())
        };

        let outcome = fetch_subscription(
            &url,
            settings.sub_fetch_proxy.as_deref(),
            &settings.sub_user_agent,
            settings.sub_fetch_timeout_ms,
            etag,
            last_mod,
            cached.as_deref(),
        )
        .await;

        let now = now_secs();
        match outcome {
            Ok(o) => {
                self.persist_success(&sub, &o, now)?;
                Ok(o.parsed)
            }
            Err(e) => {
                // 记录错误但不删除订阅（下次可恢复）
                self.persist_failure(&sub, &e)?;
                Err(e)
            }
        }
    }

    fn persist_success(&self, sub: &Subscription, o: &FetchOutcome, now: i64) -> AppResult<()> {
        // 1. 原始 YAML 落盘（304 时不重写，省 IO）
        if !o.not_modified {
            self.subs.save_raw_yaml(&sub.uid, o.raw_text.as_bytes())?;
        }

        // 2. meta 落盘
        let meta = SubscriptionMeta {
            etag: o.etag.clone(),
            last_modified: o.last_modified.clone(),
            content_length: Some(o.body_bytes),
            node_count: o.parsed.node_count() as u32,
            fetched_at: Some(now),
        };
        self.subs.save_meta(&sub.uid, &meta)?;

        // 3. 索引更新
        let mut s = sub.clone();
        s.last_update_at = Some(now);
        s.last_update_status = UpdateStatus::Ok;
        s.last_error = None;
        s.node_count = o.parsed.node_count() as u32;
        s.group_names = o.parsed.group_names.clone();
        // E_USERINFO_MISSING 不是错误，不写 last_error（契约 C.5）
        if let Some(ui) = &o.userinfo {
            s.mgmt = Some(ui.clone());
        }
        self.subs.upsert(s)?;
        Ok(())
    }

    fn persist_failure(&self, sub: &Subscription, e: &AppError) -> AppResult<()> {
        let mut s = sub.clone();
        s.last_update_status = UpdateStatus::Failed;
        // 脱敏后截断，避免把 token 写进错误信息
        s.last_error = Some(crate::domain::errors::truncate(
            &crate::util::redact::sanitize(&format!("[{}] {}", e.code.as_str(), e.message)),
            200,
        ));
        self.subs.upsert(s)?;
        Ok(())
    }

    /// 拉取全部到期订阅（并发受 `sub_fetch_concurrency` 限制）
    pub async fn fetch_due(&self, force: bool) -> Vec<(String, AppResult<ParsedSubscription>)> {
        let _guard = self.fetch_lock.lock().await;
        let now = now_secs();
        let uids = if force {
            self.subs.list().into_iter().filter(|s| s.enabled).map(|s| s.uid).collect()
        } else {
            self.due_uids(now)
        };

        if uids.is_empty() {
            return Vec::new();
        }

        let concurrency = self.settings.get().sub_fetch_concurrency.clamp(1, 4) as usize;
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let mut handles = Vec::with_capacity(uids.len());

        for uid in uids {
            let sem = Arc::clone(&sem);
            // 不能借用 self（生命周期跨 await），改为在调用方聚合
            let _ = sem;
            handles.push(uid);
        }

        let mut out = Vec::new();
        for uid in handles {
            let r = self.fetch_one(&uid, force).await;
            out.push((uid, r));
        }
        out
    }

    /// 读取所有启用订阅的原始 YAML 并生成合并配置。
    ///
    /// 返回 None 表示没有任何可用订阅。
    pub fn build_config(&self, mixed_port: u16, api_port: u16, secret: &str) -> AppResult<Option<Generated>> {
        let settings = self.settings.get();
        let active: Vec<_> = self
            .subs
            .list()
            .into_iter()
            .filter(|s| s.enabled)
            .collect();

        if active.is_empty() {
            return Ok(None);
        }

        let mut sub_nodes: Vec<SubNodes<'_>> = Vec::new();
        // 需要持有解析结果以延长生命周期
        let mut parsed_store: Vec<ParsedSubscription> = Vec::new();
        let mut names: Vec<String> = Vec::new();

        for s in &active {
            let Some(raw) = self.subs.read_raw_yaml(&s.uid)? else {
                tracing::warn!(uid = %s.uid, "订阅无本地缓存，跳过");
                continue;
            };
            match crate::subscription::parse::parse_subscription(&raw) {
                Ok(p) => {
                    names.push(s.name.clone());
                    parsed_store.push(p);
                }
                Err(e) => {
                    // 单个订阅坏了不应拖垮全部，跳过并继续
                    tracing::warn!(uid = %s.uid, err = %e, "订阅解析失败，已跳过");
                }
            }
        }

        if parsed_store.is_empty() {
            return Ok(None);
        }

        // 判断是否需要加订阅前缀：跨订阅重名时整体加前缀更清晰
        let need_prefix = has_cross_duplicates(&parsed_store) || parsed_store.len() > 1;

        for (i, p) in parsed_store.iter().enumerate() {
            let proxies: Vec<(String, serde_yaml_ng::Value)> = p
                .proxies
                .iter()
                .filter_map(|pp| {
                    pp.raw
                        .as_ref()
                        .map(|raw| (pp.name.clone(), raw.clone()))
                })
                .collect();
            sub_nodes.push(SubNodes {
                sub_name: &names[i],
                prefix: need_prefix,
                proxies,
            });
        }

        let input = GenerateInput {
            subscriptions: sub_nodes,
            mixed_port,
            api_port,
            secret: secret.to_string(),
            main_group: crate::subscription::generator::MAIN_GROUP_DEFAULT.to_string(),
            log_level: settings.log_level.to_mihomo(),
            tun_enabled: matches!(settings.proxy_mode, crate::domain::model::ProxyMode::Tun),
        };

        let generated = generate(&input)?;
        Ok(Some(generated))
    }

    /// 生成并原子写入 `runtime/config.yaml`
    pub fn write_config(&self, generated: &Generated) -> AppResult<()> {
        let path = self.paths.runtime_config();
        atomic_write(&path, generated.yaml.as_bytes())?;
        tracing::info!(nodes = generated.node_names.len(), "配置文件已写入");
        Ok(())
    }

    /// 聚合流量摘要
    pub fn traffic_summary(&self) -> crate::domain::model::TrafficSummary {
        crate::domain::model::TrafficSummary::build(&self.subs.list(), now_secs())
    }
}

/// 生成 8 位 base32 uid（小写，无易混字符）
fn generate_uid() -> String {
    use rand::Rng;
    // 去掉 l/1/0/o 等易混字符
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// 检测跨订阅是否存在重名节点
fn has_cross_duplicates(parsed: &[ParsedSubscription]) -> bool {
    use std::collections::HashSet;
    let mut seen: HashSet<&str> = HashSet::new();
    for p in parsed {
        for node in &p.proxies {
            if !seen.insert(node.name.as_str()) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::paths::Paths;

    struct Env {
        _dir: tempfile::TempDir,
        mgr: SubscriptionManager,
        paths: Arc<Paths>,
    }

    fn setup() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let paths = Arc::new(Paths::with_roots(
            dir.path().join("roaming"),
            dir.path().join("local"),
        ));
        std::fs::create_dir_all(paths.profiles_dir()).unwrap();
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        let subs = Arc::new(SubscriptionStore::load(&paths).unwrap());
        let settings = Arc::new(SettingsStore::load(&paths).unwrap());
        let mgr = SubscriptionManager::new(Arc::clone(&paths), subs, settings);
        Env {
            _dir: dir,
            mgr,
            paths,
        }
    }

    #[test]
    fn add_encrypts_url_and_never_stores_plaintext() {
        let e = setup();
        let sub = e
            .mgr
            .add("测试机场", "https://a.com/api/v1/client/subscribe?token=SECRET123")
            .unwrap();

        assert_eq!(sub.name, "测试机场");
        assert!(!sub.url_cipher.contains("SECRET123"), "密文不得含明文");
        assert!(!sub.url_hint.contains("SECRET123"), "提示串不得含 token");

        // 能解回原文
        let url = e.mgr.decrypt_url(&sub.uid).unwrap();
        assert!(url.contains("SECRET123"));
    }

    #[test]
    fn uid_is_8_chars_no_ambiguous() {
        let e = setup();
        let s = e.mgr.add("a", "https://a.com/sub").unwrap();
        assert_eq!(s.uid.len(), 8);
        assert!(!s.uid.contains('l'), "不含易混字符 l");
        assert!(!s.uid.contains('1'));
        assert!(!s.uid.contains('0'));
        assert!(!s.uid.contains('o'));
    }

    #[test]
    fn add_rejects_invalid_url() {
        let e = setup();
        assert!(e.mgr.add("x", "ftp://a.com/x").is_err());
        assert!(e.mgr.add("x", "not a url").is_err());
        assert!(e.mgr.add("", "https://a.com/sub").is_err(), "名称不能为空");
    }

    #[test]
    fn remove_and_not_found() {
        let e = setup();
        let s = e.mgr.add("a", "https://a.com/sub").unwrap();
        e.mgr.remove(&s.uid).unwrap();
        assert!(e.mgr.list().is_empty());
        assert_eq!(e.mgr.remove(&s.uid).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn set_enabled_toggles() {
        let e = setup();
        let s = e.mgr.add("a", "https://a.com/sub").unwrap();
        let s2 = e.mgr.set_enabled(&s.uid, false).unwrap();
        assert!(!s2.enabled);
        assert!(!e.mgr.list()[0].enabled);
    }

    #[test]
    fn interval_clamped_to_15_min() {
        let e = setup();
        let s = e.mgr.add("a", "https://a.com/sub").unwrap();
        let s2 = e.mgr.update_interval(&s.uid, 1).unwrap();
        assert_eq!(s2.update_interval_min, 15);
    }

    #[test]
    fn due_uids_filters_disabled() {
        let e = setup();
        let s1 = e.mgr.add("a", "https://a.com/sub1").unwrap();
        let s2 = e.mgr.add("b", "https://a.com/sub2").unwrap();
        e.mgr.set_enabled(&s2.uid, false).unwrap();

        let due = e.mgr.due_uids(now_secs());
        assert_eq!(due.len(), 1, "停用订阅不参与");
        assert_eq!(due[0], s1.uid);
    }

    #[test]
    fn build_config_none_when_no_subs() {
        let e = setup();
        let r = e.mgr.build_config(10801, 49152, "sec").unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn build_config_from_cached_yaml() {
        let e = setup();
        let s = e.mgr.add("机场A", "https://a.com/sub").unwrap();

        // 手动塞入缓存（模拟已拉取）
        let yaml = r#"
proxies:
  - name: "香港 01"
    type: ss
    server: a.example.com
    port: 8388
    cipher: aes-256-gcm
    password: "p"
"#;
        e.mgr.subs.save_raw_yaml(&s.uid, yaml.as_bytes()).unwrap();

        let gen = e.mgr.build_config(10801, 49152, "secret").unwrap().unwrap();
        assert_eq!(gen.node_names.len(), 1);
        assert!(gen.yaml.contains("mixed-port: 10801"));
        assert!(gen.yaml.contains("select"));
    }

    #[test]
    fn build_config_writes_file() {
        let e = setup();
        let s = e.mgr.add("机场A", "https://a.com/sub").unwrap();
        let yaml = "proxies:\n  - name: n1\n    type: ss\n    server: a.com\n    port: 1\n";
        e.mgr.subs.save_raw_yaml(&s.uid, yaml.as_bytes()).unwrap();

        let gen = e.mgr.build_config(10801, 49152, "sec").unwrap().unwrap();
        e.mgr.write_config(&gen).unwrap();
        assert!(e.paths.runtime_config().exists());

        let content = std::fs::read_to_string(e.paths.runtime_config()).unwrap();
        assert!(content.contains("mixed-port"));
    }

    #[test]
    fn build_config_skips_disabled_subs() {
        let e = setup();
        let s = e.mgr.add("A", "https://a.com/1").unwrap();
        e.mgr.subs
            .save_raw_yaml(&s.uid, b"proxies:\n  - name: n1\n    type: ss\n    server: a.com\n    port: 1\n")
            .unwrap();
        e.mgr.set_enabled(&s.uid, false).unwrap();

        assert!(e.mgr.build_config(10801, 49152, "s").unwrap().is_none());
    }

    #[test]
    fn traffic_summary_empty_when_no_subs() {
        let e = setup();
        let t = e.mgr.traffic_summary();
        assert_eq!(t.items.len(), 0);
        assert_eq!(t.aggregate.total, None);
    }

    #[test]
    fn duplicate_detection_across_subs() {
        use crate::subscription::parse::parse_subscription;
        let a = parse_subscription("proxies:\n  - name: same\n    type: ss\n").unwrap();
        let b = parse_subscription("proxies:\n  - name: same\n    type: ss\n").unwrap();
        assert!(has_cross_duplicates(&[a, b]));

        let c = parse_subscription("proxies:\n  - name: x\n    type: ss\n").unwrap();
        let d = parse_subscription("proxies:\n  - name: y\n    type: ss\n").unwrap();
        assert!(!has_cross_duplicates(&[c, d]));
    }
}
