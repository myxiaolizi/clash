//! 设置与订阅索引的持久化。全部走原子写 + 损坏回退。

use std::sync::RwLock;

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{
    AppSettings, Subscription, SubscriptionMeta, SETTINGS_SCHEMA_VERSION,
};
use crate::system::paths::{atomic_write, read_json_recover, write_json, Paths};

/// 设置存储：内存缓存 + 落盘
pub struct SettingsStore {
    path: std::path::PathBuf,
    inner: RwLock<AppSettings>,
}

impl SettingsStore {
    pub fn load(paths: &Paths) -> AppResult<Self> {
        let path = paths.settings_file();
        let existed = path.exists();
        let loaded: Option<AppSettings> = read_json_recover(&path)?;
        let mut settings = loaded.unwrap_or_default();

        // schema 迁移：低版本补默认值
        let before = settings.schema_version;
        if settings.schema_version < SETTINGS_SCHEMA_VERSION {
            settings = migrate(settings);
            tracing::info!(from = before, to = settings.schema_version, "设置已迁移");
        }
        settings.sanitize();

        let store = Self {
            path,
            inner: RwLock::new(settings),
        };

        // 落盘时机（**两个条件都要**）：
        // 1) 文件原本不存在 —— 首次运行必须把默认值写出来，否则用户不知道数据在哪，
        //    外部工具（备份/排障）也读不到。仅靠 `before != CURRENT` 判断是错的：
        //    `AppSettings::default()` 的 schema_version 就是 CURRENT，条件永不成立。
        // 2) 版本发生迁移 —— 磁盘上还是旧结构，需要回写。
        if !existed || before != SETTINGS_SCHEMA_VERSION {
            if let Err(e) = store.save() {
                // 写失败不该阻止应用启动（可能只是只读目录），但要留痕
                tracing::warn!(err = %e, "首次写入设置失败，继续使用内存默认值");
            }
        }
        Ok(store)
    }

    pub fn get(&self) -> AppSettings {
        self.inner
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// 读取-修改-写回，保证原子性
    pub fn update<F>(&self, f: F) -> AppResult<AppSettings>
    where
        F: FnOnce(&mut AppSettings),
    {
        let snapshot = {
            let mut w = self
                .inner
                .write()
                .map_err(|_| AppError::msg(ErrorCode::Internal, "设置锁中毒"))?;
            f(&mut w);
            w.sanitize();
            w.clone()
        };
        self.save_with(&snapshot)?;
        Ok(snapshot)
    }

    pub fn save(&self) -> AppResult<()> {
        let s = self.get();
        self.save_with(&s)
    }

    fn save_with(&self, s: &AppSettings) -> AppResult<()> {
        write_json(&self.path, s)
    }
}

/// 低版本设置 → 当前版本
fn migrate(mut s: AppSettings) -> AppSettings {
    let from = s.schema_version;

    // v1 → v2：新增 compliance_accepted_at（Option，默认 None 即可，无需特殊处理）
    if from < 2 {
        if s.compliance_accepted_at.is_none() {
            tracing::debug!("v1→v2: compliance_accepted_at 缺省为 None");
        }
    }

    // v2 → v3：新增 mihomo_mode 与失败转移字段。
    //
    // 这些字段都有 `#[serde(default)]`，老文件缺字段时会被自动补上默认值，
    // 所以不需要逐字段迁移。但**必须跑一次 sanitize**：
    // 万一用户手改过 settings.json 塞进越界值（比如 retry_interval_sec=0
    // 会造成忙轮询），升级后要在加载阶段就纠正掉，而不是等到运行时才出问题。
    if from < 3 {
        // mihomo_mode / 失败转移字段都有 serde default，缺字段自动补默认值，
        // 不用逐字段迁移；但要跑一次 sanitize 纠正手改文件塞进的越界值。
        s.switch_policy.sanitize();
        tracing::debug!(
            mode = s.mihomo_mode.as_str(),
            "v2→v3: 已补全分流模式与失败转移默认值"
        );
    }

    s.schema_version = SETTINGS_SCHEMA_VERSION;
    s
}

/// 订阅索引存储
pub struct SubscriptionStore {
    index_path: std::path::PathBuf,
    profiles_dir: std::path::PathBuf,
    cache: RwLock<Vec<Subscription>>,
}

impl SubscriptionStore {
    pub fn load(paths: &Paths) -> AppResult<Self> {
        let index_path = paths.profiles_index();
        let list: Option<Vec<Subscription>> = read_json_recover(&index_path)?;
        Ok(Self {
            index_path,
            profiles_dir: paths.profiles_dir(),
            cache: RwLock::new(list.unwrap_or_default()),
        })
    }

    pub fn list(&self) -> Vec<Subscription> {
        self.cache.read().map(|v| v.clone()).unwrap_or_default()
    }

    pub fn get(&self, uid: &str) -> Option<Subscription> {
        self.cache
            .read()
            .ok()
            .and_then(|v| v.iter().find(|s| s.uid == uid).cloned())
    }

    pub fn upsert(&self, sub: Subscription) -> AppResult<()> {
        let snapshot = {
            let mut w = self
                .cache
                .write()
                .map_err(|_| AppError::msg(ErrorCode::Internal, "订阅锁中毒"))?;
            match w.iter_mut().find(|s| s.uid == sub.uid) {
                Some(existing) => *existing = sub,
                None => w.push(sub),
            }
            w.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.uid.cmp(&b.uid)));
            w.clone()
        };
        write_json(&self.index_path, &snapshot)
    }

    pub fn remove(&self, uid: &str) -> AppResult<()> {
        let snapshot = {
            let mut w = self
                .cache
                .write()
                .map_err(|_| AppError::msg(ErrorCode::Internal, "订阅锁中毒"))?;
            w.retain(|s| s.uid != uid);
            w.clone()
        };
        write_json(&self.index_path, &snapshot)?;
        // 连带清理该订阅的文件
        let yaml = self.profiles_dir.join(format!("{uid}.yaml"));
        let meta = self.profiles_dir.join(format!("{uid}.meta.json"));
        let _ = std::fs::remove_file(yaml);
        let _ = std::fs::remove_file(meta);
        Ok(())
    }

    /// 保存订阅原始 YAML（原样保存 HTTP 响应体）
    pub fn save_raw_yaml(&self, uid: &str, body: &[u8]) -> AppResult<()> {
        let p = self.profiles_dir.join(format!("{uid}.yaml"));
        atomic_write(&p, body)
    }

    pub fn read_raw_yaml(&self, uid: &str) -> AppResult<Option<String>> {
        let p = self.profiles_dir.join(format!("{uid}.yaml"));
        match std::fs::read(&p) {
            Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::with_detail(ErrorCode::StoreIo, e)),
        }
    }

    pub fn save_meta(&self, uid: &str, meta: &SubscriptionMeta) -> AppResult<()> {
        let p = self.profiles_dir.join(format!("{uid}.meta.json"));
        write_json(&p, meta)
    }

    pub fn read_meta(&self, uid: &str) -> AppResult<SubscriptionMeta> {
        let p = self.profiles_dir.join(format!("{uid}.meta.json"));
        Ok(read_json_recover::<SubscriptionMeta>(&p)?.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::paths::Paths;

    fn setup() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        std::fs::create_dir_all(paths.profiles_dir()).unwrap();
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();
        (dir, paths)
    }

    #[test]
    fn settings_defaults_then_persist() {
        let (_d, paths) = setup();
        let store = SettingsStore::load(&paths).unwrap();
        assert_eq!(store.get().schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(store.get().sub_default_interval_min, 360);

        store.update(|s| s.sub_default_interval_min = 120).unwrap();

        // 重新加载应读回
        let store2 = SettingsStore::load(&paths).unwrap();
        assert_eq!(store2.get().sub_default_interval_min, 120);
    }

    #[test]
    fn first_run_writes_settings_file() {
        // 回归测试：曾经因为 `AppSettings::default().schema_version` 已等于
        // CURRENT，`before != CURRENT` 永不成立，首次运行从不落盘，
        // 导致 settings.json 缺失、窗口显示逻辑读取到空配置。
        let (_d, paths) = setup();
        let f = paths.settings_file();
        assert!(!f.exists(), "前置条件：文件尚不存在");

        let _store = SettingsStore::load(&paths).unwrap();

        assert!(f.exists(), "首次加载必须把默认设置写出来");
        let raw = std::fs::read_to_string(&f).unwrap();
        assert!(raw.contains("schema_version"), "落盘内容应含 schema_version");
        assert!(raw.len() > 50, "不应写入空对象");
    }

    #[test]
    fn settings_update_sanitizes_on_write() {
        let (_d, paths) = setup();
        let store = SettingsStore::load(&paths).unwrap();
        let s = store.update(|s| s.sub_default_interval_min = 1).unwrap();
        assert_eq!(s.sub_default_interval_min, 15, "写入前必须 sanitize");
        // 磁盘上也应是 15
        let store2 = SettingsStore::load(&paths).unwrap();
        assert_eq!(store2.get().sub_default_interval_min, 15);
    }

    /// 构造一份"真实"的 v2 settings.json：字段齐全，但删掉 v3 才有的键。
    ///
    /// 不能手写两三行 JSON —— `AppSettings` 大部分字段没有 serde default，
    /// 缺字段会整体反序列化失败并回退到默认值，那样测的就不是迁移而是兜底了。
    fn v2_json() -> String {
        let mut v = serde_json::to_value(AppSettings::default()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.insert("schema_version".into(), serde_json::json!(2));
        obj.remove("mihomo_mode");
        if let Some(p) = obj.get_mut("switch_policy").and_then(|p| p.as_object_mut()) {
            for k in [
                "on_failure_action",
                "failover_enabled",
                "retry_interval_sec",
                "max_retries",
                "dwell_after_switch_sec",
                "prefer_healthy_only",
            ] {
                p.remove(k);
            }
        }
        serde_json::to_string(&v).unwrap()
    }

    #[test]
    fn migrate_v2_to_v3_fills_new_fields() {
        let (_d, paths) = setup();
        std::fs::write(paths.settings_file(), v2_json()).unwrap();

        let store = SettingsStore::load(&paths).unwrap();
        let s = store.get();

        assert_eq!(s.schema_version, SETTINGS_SCHEMA_VERSION);
        // 新字段走 serde default
        assert_eq!(s.mihomo_mode, crate::domain::model::MihomoMode::Rule);
        assert!(s.switch_policy.failover_enabled);
        assert_eq!(
            s.switch_policy.on_failure_action,
            crate::domain::model::OnFailureAction::Switch
        );
        // 老字段必须原样保留（证明是真读进来了，不是兜底默认值）
        assert_eq!(s.sub_default_interval_min, 360);
    }

    #[test]
    fn migrate_sanitizes_hand_edited_out_of_range() {
        let (_d, paths) = setup();
        // 用户手改 settings.json 塞了会忙轮询的 0 和超大的重试次数。
        //
        // 注意不能只留两个字段：SwitchPolicy 大部分字段没有 serde default，
        // 缺字段会让整个 AppSettings 反序列化失败并回退默认值 ——
        // 那样测到的是"兜底"而不是"纠正越界值"。
        let mut v: serde_json::Value = serde_json::from_str(&v2_json()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("schema_version".into(), serde_json::json!(2));
        {
            let p = v
                .get_mut("switch_policy")
                .and_then(|p| p.as_object_mut())
                .unwrap();
            p.insert("retry_interval_sec".into(), serde_json::json!(0));
            p.insert("max_retries".into(), serde_json::json!(999));
        }
        std::fs::write(paths.settings_file(), serde_json::to_string(&v).unwrap()).unwrap();

        let store = SettingsStore::load(&paths).unwrap();
        let p = &store.get().switch_policy;
        assert_eq!(p.retry_interval_sec, 1, "加载时必须纠正 0");
        assert_eq!(p.max_retries, 10, "加载时必须夹住 999");
    }

    #[test]
    fn migrate_keeps_existing_mihomo_mode() {
        let (_d, paths) = setup();
        let mut v: serde_json::Value = serde_json::from_str(&v2_json()).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("mihomo_mode".into(), serde_json::json!("global"));
        std::fs::write(paths.settings_file(), serde_json::to_string(&v).unwrap()).unwrap();

        let store = SettingsStore::load(&paths).unwrap();
        assert_eq!(
            store.get().mihomo_mode,
            crate::domain::model::MihomoMode::Global,
            "已有值不得被迁移覆盖"
        );
    }

    #[test]
    fn corrupt_settings_falls_back_to_default() {
        let (_d, paths) = setup();
        std::fs::write(paths.settings_file(), b"{{{bad").unwrap();
        let store = SettingsStore::load(&paths).unwrap();
        assert_eq!(store.get().schema_version, SETTINGS_SCHEMA_VERSION);
        // 坏文件应被备份
        let backups: Vec<_> = std::fs::read_dir(&paths.roaming)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
            .collect();
        assert!(!backups.is_empty());
    }

    #[test]
    fn subscription_crud_and_file_cleanup() {
        let (_d, paths) = setup();
        let store = SubscriptionStore::load(&paths).unwrap();

        let sub = Subscription::new(
            "u1".into(),
            "机场A".into(),
            "cipher".into(),
            "https://a.com/api/v1/******".into(),
            360,
        );
        store.upsert(sub.clone()).unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(store.get("u1").is_some());

        store.save_raw_yaml("u1", b"proxies: []").unwrap();
        assert_eq!(store.read_raw_yaml("u1").unwrap().unwrap(), "proxies: []");

        store.remove("u1").unwrap();
        assert!(store.list().is_empty());
        assert!(!paths.profile_yaml("u1").exists(), "关联文件应被清理");
    }

    #[test]
    fn subscription_sorted_by_sort_order() {
        let (_d, paths) = setup();
        let store = SubscriptionStore::load(&paths).unwrap();
        for (i, uid) in ["c", "a", "b"].iter().enumerate() {
            let mut s = Subscription::new(
                uid.to_string(),
                uid.to_string(),
                "x".into(),
                "h".into(),
                360,
            );
            s.sort_order = match *uid {
                "c" => 3,
                "a" => 1,
                _ => 2,
            };
            let _ = i;
            store.upsert(s).unwrap();
        }
        let uids: Vec<_> = store.list().into_iter().map(|s| s.uid).collect();
        assert_eq!(uids, vec!["a", "b", "c"]);
    }

    #[test]
    fn meta_roundtrip() {
        let (_d, paths) = setup();
        let store = SubscriptionStore::load(&paths).unwrap();
        let meta = SubscriptionMeta {
            etag: Some("W/\"abc\"".into()),
            last_modified: Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
            content_length: Some(12345),
            node_count: 42,
            fetched_at: Some(1000),
        };
        store.save_meta("u1", &meta).unwrap();
        let back = store.read_meta("u1").unwrap();
        assert_eq!(back.etag, meta.etag);
        assert_eq!(back.node_count, 42);
        // 不存在时返回默认
        let empty = store.read_meta("nope").unwrap();
        assert!(empty.etag.is_none());
    }
}
