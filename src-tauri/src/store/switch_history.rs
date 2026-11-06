//! 切换历史持久化（`runtime/switch-history.json`，保留最近 200 条）。

use std::path::PathBuf;
use std::sync::RwLock;

use crate::domain::errors::{AppResult, ErrorCode, AppError};
use crate::domain::model::SwitchRecord;
use crate::system::paths::{read_json_recover, write_json, Paths};

pub struct SwitchHistoryStore {
    path: PathBuf,
    cache: RwLock<Vec<SwitchRecord>>,
}

impl SwitchHistoryStore {
    pub fn load(paths: &Paths) -> AppResult<Self> {
        let path = paths.switch_history();
        let list: Option<Vec<SwitchRecord>> = read_json_recover(&path)?;
        let mut v = list.unwrap_or_default();
        // 防御：文件可能被外部改大
        if v.len() > SwitchRecord::MAX_HISTORY {
            let drop_n = v.len() - SwitchRecord::MAX_HISTORY;
            v.drain(0..drop_n);
        }
        Ok(Self {
            path,
            cache: RwLock::new(v),
        })
    }

    /// 最近 N 条，**新的在前**
    pub fn recent(&self, limit: usize) -> Vec<SwitchRecord> {
        let guard = match self.cache.read() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        guard.iter().rev().take(limit).cloned().collect()
    }

    /// 追加一条并落盘
    pub fn push(&self, rec: SwitchRecord) -> AppResult<()> {
        let snapshot = {
            let mut w = self
                .cache
                .write()
                .map_err(|_| AppError::msg(ErrorCode::Internal, "切换历史锁中毒"))?;
            SwitchRecord::push_capped(&mut w, rec);
            w.clone()
        };
        write_json(&self.path, &snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::SwitchReason;
    use crate::system::paths::Paths;

    fn rec(at: i64, to: &str) -> SwitchRecord {
        SwitchRecord {
            at,
            group: "Proxy".into(),
            from_node: Some("old".into()),
            to_node: to.into(),
            from_delay: Some(300),
            to_delay: Some(50),
            reason: SwitchReason::Best,
        }
    }

    #[test]
    fn push_and_read_recent_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        let store = SwitchHistoryStore::load(&paths).unwrap();
        store.push(rec(1, "a")).unwrap();
        store.push(rec(2, "b")).unwrap();
        store.push(rec(3, "c")).unwrap();

        let recent = store.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].to_node, "c", "新的在前");
        assert_eq!(recent[1].to_node, "b");
    }

    #[test]
    fn history_persists_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        {
            let store = SwitchHistoryStore::load(&paths).unwrap();
            store.push(rec(1, "a")).unwrap();
        }
        let store2 = SwitchHistoryStore::load(&paths).unwrap();
        assert_eq!(store2.recent(10).len(), 1);
    }

    #[test]
    fn history_capped_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        let store = SwitchHistoryStore::load(&paths).unwrap();
        for i in 0..250 {
            store.push(rec(i, "n")).unwrap();
        }
        let all = store.recent(1000);
        assert_eq!(all.len(), SwitchRecord::MAX_HISTORY);
        assert_eq!(all[0].at, 249, "最新的在最前");
    }

    #[test]
    fn oversized_file_is_trimmed_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        // 手写一个超长文件模拟外部损坏/手工编辑
        let big: Vec<SwitchRecord> = (0..400).map(|i| rec(i, "n")).collect();
        write_json(&paths.switch_history(), &big).unwrap();

        let store = SwitchHistoryStore::load(&paths).unwrap();
        assert_eq!(store.recent(1000).len(), SwitchRecord::MAX_HISTORY);
    }
}
