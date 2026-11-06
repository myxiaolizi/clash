//! 测速编排（P4）。
//!
//! 并发策略：信号量限制并发（默认 8），避免一次性打满机场或本地连接数。
//! 单节点探测走内核 API `/proxies/{name}/delay`，
//! **不用** `/group/{g}/delay`（附录 A 明确禁用：会清空自动组的固定选择）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use tokio::sync::{broadcast, Semaphore};

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{
    ProbeResult, SelectableGroup, SpeedtestRun, SpeedtestState, SpeedtestTrigger, 
    SwitchRecord,
};
use crate::domain::policy::{
    decide, is_selector, rank_candidates_with_priority, update_failure_counters, DecisionCtx,
    ExcludeRules, FailureCounter, KIND_SELECTOR,
};
use crate::kernel::SelectorApi;
use crate::speedtest::rank::RankOutcome;
use crate::store::switch_history::SwitchHistoryStore;
use crate::util::time::now_secs;

/// 测速事件
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedtestEvent {
    pub run_id: String,
    pub finished: u32,
    pub total: u32,
    pub ok_count: u32,
    pub at: i64,
}

/// 自动切换事件
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchEvent {
    pub group: String,
    pub from_node: Option<String>,
    pub to_node: String,
    pub from_delay: Option<u32>,
    pub to_delay: Option<u32>,
    pub reason: String,
    pub at: i64,
}

/// 测速运行状态（内存）
pub struct SpeedtestEngine {
    api: Arc<dyn SelectorApi>,
    history: Arc<SwitchHistoryStore>,
    running: parking_lot::Mutex<Option<String>>,
    last_run: parking_lot::RwLock<Option<SpeedtestRun>>,
    failures: parking_lot::Mutex<FailureCounter>,
    /// 每组的最近一次切换时间（min_dwell）
    last_switch: parking_lot::Mutex<std::collections::HashMap<String, i64>>,
    /// 手动覆盖锁定到期时刻
    manual_lock: parking_lot::Mutex<std::collections::HashMap<String, i64>>,
    events: broadcast::Sender<SpeedtestEvent>,
    switch_events: broadcast::Sender<SwitchEvent>,
    /// 当前探测参数（由设置注入）
    cfg: parking_lot::RwLock<ProbeConfig>,
}

#[derive(Debug, Clone)]
pub struct ProbeConfig {
    pub probe_url: String,
    pub timeout_ms: u32,
    pub concurrency: u8,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            probe_url: "http://www.gstatic.com/generate_204".into(),
            timeout_ms: 3000,
            concurrency: 8,
        }
    }
}

impl SpeedtestEngine {
    pub fn new(
        api: Arc<dyn SelectorApi>,
        history: Arc<SwitchHistoryStore>,
        events: broadcast::Sender<SpeedtestEvent>,
        switch_events: broadcast::Sender<SwitchEvent>,
    ) -> Arc<Self> {
        Arc::new(Self {
            api,
            history,
            running: parking_lot::Mutex::new(None),
            last_run: parking_lot::RwLock::new(None),
            failures: parking_lot::Mutex::new(FailureCounter::new()),
            last_switch: parking_lot::Mutex::new(std::collections::HashMap::new()),
            manual_lock: parking_lot::Mutex::new(std::collections::HashMap::new()),
            events,
            switch_events,
            cfg: parking_lot::RwLock::new(ProbeConfig::default()),
        })
    }

    pub fn update_config(&self, cfg: ProbeConfig) {
        *self.cfg.write() = cfg;
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().is_some()
    }

    pub fn last_run(&self) -> Option<SpeedtestRun> {
        self.last_run.read().clone()
    }

    /// 记录用户手动选节点（触发手动覆盖锁定）
    pub fn note_manual_switch(&self, group: &str, override_min: u32) {
        let now = now_secs();
        let until = now.saturating_add((override_min as i64) * 60);
        self.manual_lock.lock().insert(group.to_string(), until);
        // 也更新 last_switch，让 min_dwell 从这个时刻起算
        self.last_switch.lock().insert(group.to_string(), now);
        tracing::info!(group, until, "手动切换已记录，自动切换将暂停");
    }

    /// 执行一次完整测速（所有可切换组的成员去重后探测）
    pub async fn run(&self, trigger: SpeedtestTrigger) -> AppResult<SpeedtestRun> {
        // 防重入
        {
            let mut r = self.running.lock();
            if r.is_some() {
                return Err(AppError::new(ErrorCode::SpeedtestRunning));
            }
            *r = Some("pending".to_string());
        }

        let result = self.run_inner(trigger).await;

        // 无论成功失败都要释放 running 标记，否则永远无法再次测速
        *self.running.lock() = None;
        match &result {
            Ok(run) => {
                *self.last_run.write() = Some(run.clone());
            }
            Err(e) => tracing::warn!(err = %e, "测速运行失败"),
        }
        result
    }

    async fn run_inner(&self, trigger: SpeedtestTrigger) -> AppResult<SpeedtestRun> {
        let groups = self.api.proxies_selectable().await.map_err(AppError::from)?;
        if groups.is_empty() {
            return Err(AppError::new(ErrorCode::SpeedtestNoNode));
        }

        // 汇总所有待测节点（去重）
        let mut all_nodes: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for g in &groups {
            for m in &g.members {
                if seen.insert(m.clone()) {
                    all_nodes.push(m.clone());
                }
            }
        }
        if all_nodes.is_empty() {
            return Err(AppError::new(ErrorCode::SpeedtestNoNode));
        }

        let cfg = self.cfg.read().clone();
        let run_id = uuid::Uuid::new_v4().to_string();
        *self.running.lock() = Some(run_id.clone());

        let total = all_nodes.len() as u32;
        let run = Arc::new(parking_lot::Mutex::new(SpeedtestRun::new(
            run_id.clone(),
            trigger,
            total,
            now_secs(),
        )));
        let finished = Arc::new(AtomicU32::new(0));
        let ok_count = Arc::new(AtomicU32::new(0));

        let sem = Arc::new(Semaphore::new(cfg.concurrency.clamp(1, 16) as usize));
        let mut handles = Vec::with_capacity(all_nodes.len());

        for node in all_nodes {
            let sem = Arc::clone(&sem);
            let api = Arc::clone(&self.api);
            let run = Arc::clone(&run);
            let finished = Arc::clone(&finished);
            let ok_count = Arc::clone(&ok_count);
            let events = self.events.clone();
            let probe_url = cfg.probe_url.clone();
            let timeout_ms = cfg.timeout_ms;
            let run_id_c = run_id.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await;
                let at = now_secs();
                let node_name = node.clone();

                let result = match api.proxy_delay(&node, &probe_url, timeout_ms).await {
                    Ok(d) => {
                        ok_count.fetch_add(1, Ordering::Relaxed);
                        ProbeResult::ok(node, node_name, d, at)
                    }
                    Err(crate::domain::model::ApiError::NotFound(_)) => {
                        ProbeResult::fail(node, node_name, "节点不存在".into(), at)
                    }
                    Err(crate::domain::model::ApiError::Timeout) => {
                        ProbeResult::fail(node, node_name, "超时".into(), at)
                    }
                    Err(e) => ProbeResult::fail(node, node_name, format!("{e}"), at),
                };

                let done = finished.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(mut g) = run.try_lock() {
                    g.results.push(result);
                    g.finished = done;
                }

                // 每秒最多 4 条事件的约束由发送端节流保证（这里只发里程碑）
                if done == 1 || done % 5 == 0 {
                    let _ = events.send(SpeedtestEvent {
                        run_id: run_id_c,
                        finished: done,
                        total,
                        ok_count: ok_count.load(Ordering::Relaxed),
                        at: now_secs(),
                    });
                }
            }));
        }

        for h in handles {
            let _ = h.await;
        }

        let mut final_run = run.lock().clone();
        final_run.state = SpeedtestState::Done;
        final_run.finished = finished.load(Ordering::Relaxed);
        final_run.ok_count = ok_count.load(Ordering::Relaxed);
        final_run.finished_at = Some(now_secs());

        let _ = self.events.send(SpeedtestEvent {
            run_id: run_id.clone(),
            finished: final_run.finished,
            total,
            ok_count: final_run.ok_count,
            at: now_secs(),
        });

        Ok(final_run)
    }

    /// 依据最近一次测速结果做自动切换决策（P4 核心）
    pub async fn auto_switch(
        &self,
        policy: &crate::domain::model::SwitchPolicy,
        probe_url: &str,
        timeout_ms: u32,
    ) -> AppResult<Vec<SwitchRecord>> {
        if !policy.enabled {
            return Ok(Vec::new());
        }

        let groups = self.api.proxies_selectable().await.map_err(AppError::from)?;
        let rules = ExcludeRules::from_policy(policy);
        let now = now_secs();
        let mut records = Vec::new();

        // 需要探测当前节点延迟（失败计数依据）
        for g in &groups {
            // 双保险：即使 proxies_selectable 已过滤，这里再核一次 kind
            if !is_selector(KIND_SELECTOR) {
                continue;
            }

            let info = match self.api.proxy_group(&g.name).await {
                Ok(i) => i,
                // 组消失（订阅更新后重建）属正常，跳过
                Err(crate::domain::model::ApiError::NotFound(_)) => continue,
                Err(e) => {
                    tracing::warn!(group = %g.name, err = %e, "读取组信息失败");
                    continue;
                }
            };

            // 组类型必须精确匹配 Selector
            if !is_selector(&info.kind) {
                tracing::debug!(group = %g.name, kind = %info.kind, "跳过非 Selector 组");
                continue;
            }

            let current = info.now.clone();
            let Some(current_name) = current.clone() else {
                continue;
            };

            // 探测所有成员
            let probes = self.probe_members(&info.all, probe_url, timeout_ms).await;

            // 更新失败计数
            {
                let mut f = self.failures.lock();
                update_failure_counters(&mut f, &probes);
            }

            // 关键：守卫必须在 await 前丢弃。
            // parking_lot 的 MutexGuard 不是 Send，跨 await 会让整个 future 变 !Send，
            // Tauri 命令（要求 Send）就无法调用本函数。
            let current_failures = {
                let counters = self.failures.lock();
                counters.get(&current_name).copied().unwrap_or(0)
            };

            let current_delay = probes
                .iter()
                .find(|p| p.node_key == current_name)
                .and_then(|p| p.delay_ms);

            let candidates = rank_candidates_with_priority(
                &probes,
                &rules,
                &policy.prefer_region_priority,
            );

            let last_switch_at = self.last_switch.lock().get(&g.name).copied();
            let manual_lock_until = self.manual_lock.lock().get(&g.name).copied();

            let ctx = DecisionCtx {
                policy,
                group: &SelectableGroup {
                    name: info.name.clone(),
                    members: info.all.clone(),
                    now: info.now.clone(),
                },
                current: Some(&current_name),
                current_delay,
                current_failures,
                candidates: &candidates,
                last_switch_at,
                manual_lock_until,
                now,
            };

            match decide(&ctx) {
                crate::domain::policy::Decision::Hold(reason) => {
                    tracing::debug!(group = %g.name, reason = %reason.as_str(), "保持当前节点");
                }
                crate::domain::policy::Decision::Switch {
                    to_node,
                    to_delay,
                    from_delay,
                    reason,
                } => {
                    match self.api.set_group_selected(&g.name, &to_node).await {
                        Ok(()) => {
                            self.last_switch.lock().insert(g.name.clone(), now);
                            let rec = SwitchRecord {
                                at: now,
                                group: g.name.clone(),
                                from_node: Some(current_name.clone()),
                                to_node: to_node.clone(),
                                from_delay,
                                to_delay,
                                reason,
                            };
                            if let Err(e) = self.history.push(rec.clone()) {
                                tracing::warn!(err = %e, "写切换历史失败");
                            }
                            let _ = self.switch_events.send(SwitchEvent {
                                group: g.name.clone(),
                                from_node: Some(current_name),
                                to_node,
                                from_delay,
                                to_delay,
                                reason: format!("{:?}", rec.reason).to_lowercase(),
                                at: now,
                            });
                            tracing::info!(
                                group = %g.name,
                                reason = ?rec.reason,
                                "自动切换完成"
                            );
                            records.push(rec);
                        }
                        Err(e) => {
                            tracing::warn!(group = %g.name, err = %e, "切换失败");
                        }
                    }
                }
            }
        }

        Ok(records)
    }

    /// 并发探测一批成员
    async fn probe_members(
        &self,
        members: &[String],
        probe_url: &str,
        timeout_ms: u32,
    ) -> Vec<ProbeResult> {
        let cfg = self.cfg.read().clone();
        let sem = Arc::new(Semaphore::new(cfg.concurrency.clamp(1, 16) as usize));
        let mut handles = Vec::with_capacity(members.len());

        for m in members {
            // 内置名不发探测（内核会 400）
            if ExcludeRules::is_builtin(m) {
                continue;
            }
            let api = Arc::clone(&self.api);
            let node = m.clone();
            let url = probe_url.to_string();
            let sem = Arc::clone(&sem);
            handles.push(tokio::spawn(async move {
                let _p = sem.acquire().await;
                let at = now_secs();
                match api.proxy_delay(&node, &url, timeout_ms).await {
                    Ok(d) => ProbeResult::ok(node.clone(), node, d, at),
                    Err(crate::domain::model::ApiError::Timeout) => {
                        ProbeResult::fail(node.clone(), node, "超时".into(), at)
                    }
                    Err(e) => ProbeResult::fail(node.clone(), node, format!("{e}"), at),
                }
            }));
        }

        let mut out = Vec::with_capacity(handles.len());
        for h in handles {
            if let Ok(r) = h.await {
                out.push(r);
            }
        }
        out
    }

    /// 生成排序结果供 UI 展示
    pub fn rank_view(
        &self,
        results: &[ProbeResult],
        policy: &crate::domain::model::SwitchPolicy,
    ) -> RankOutcome {
        rank_candidates_with_priority(
            results,
            &ExcludeRules::from_policy(policy),
            &policy.prefer_region_priority,
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::{ApiError, GroupInfo, SwitchReason};
    use crate::kernel::api::ApiFuture;

    /// 假 API：完全可控，用于验证编排逻辑
    struct FakeApi {
        groups: Vec<SelectableGroup>,
        kind: String,
        delays: std::collections::HashMap<String, Result<u32, ApiError>>,
        selected: parking_lot::Mutex<Vec<(String, String)>>,
    }

    impl FakeApi {
        fn new(groups: Vec<SelectableGroup>) -> Self {
            Self {
                groups,
                kind: KIND_SELECTOR.to_string(),
                delays: std::collections::HashMap::new(),
                selected: parking_lot::Mutex::new(Vec::new()),
            }
        }

        fn with_delays(mut self, d: impl IntoIterator<Item = (String, Result<u32, ApiError>)>) -> Self {
            self.delays = d.into_iter().collect();
            self
        }
    }

    impl SelectorApi for FakeApi {
        fn proxies_selectable(&self) -> ApiFuture<'_, Vec<SelectableGroup>> {
            let g = self.groups.clone();
            Box::pin(async move { Ok(g) })
        }
        fn proxy_group(&self, group: &str) -> ApiFuture<'_, GroupInfo> {
            let g = self
                .groups
                .iter()
                .find(|x| x.name == group)
                .cloned()
                .ok_or_else(|| ApiError::NotFound(group.to_string()));
            let kind = self.kind.clone();
            Box::pin(async move {
                let g = g?;
                Ok(GroupInfo {
                    name: g.name,
                    kind,
                    now: g.now,
                    all: g.members,
                    test_url: None,
                    hidden: false,
                })
            })
        }
        fn set_group_selected(&self, group: &str, node: &str) -> ApiFuture<'_, ()> {
            self.selected
                .lock()
                .push((group.to_string(), node.to_string()));
            Box::pin(async { Ok(()) })
        }
        fn proxy_delay(&self, node: &str, _u: &str, _t: u32) -> ApiFuture<'_, u32> {
            let r = self.delays.get(node).cloned().unwrap_or(Ok(100));
            Box::pin(async move { r })
        }
        fn version(&self) -> ApiFuture<'_, String> {
            Box::pin(async { Ok("v1.19.33".into()) })
        }
    }

    fn engine(api: Arc<dyn SelectorApi>) -> (Arc<SpeedtestEngine>, Arc<SwitchHistoryStore>) {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::system::paths::Paths::with_roots(
            dir.path().join("r"),
            dir.path().join("l"),
        );
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();
        let hist = Arc::new(SwitchHistoryStore::load(&paths).unwrap());
        let (tx, _) = broadcast::channel(64);
        let (sx, _) = broadcast::channel(64);
        let e = SpeedtestEngine::new(api, Arc::clone(&hist), tx, sx);
        // 目录必须存活，泄漏 tempdir 以简化测试
        std::mem::forget(dir);
        (e, hist)
    }

    fn grp(name: &str, now: &str, members: &[&str]) -> SelectableGroup {
        SelectableGroup {
            name: name.into(),
            members: members.iter().map(|s| s.to_string()).collect(),
            now: Some(now.into()),
        }
    }

    #[tokio::test]
    async fn speedtest_runs_and_collects_results() {
        let api = Arc::new(FakeApi::new(vec![grp("Proxy", "a", &["a", "b"])]));
        let (e, _h) = engine(api);
        let run = e.run(SpeedtestTrigger::Manual).await.unwrap();
        assert_eq!(run.total, 2);
        assert_eq!(run.results.len(), 2);
        assert_eq!(run.state, SpeedtestState::Done);
        assert_eq!(run.ok_count, 2);
    }

    #[tokio::test]
    async fn speedtest_rejects_no_groups() {
        let api = Arc::new(FakeApi::new(vec![]));
        let (e, _h) = engine(api);
        let err = e.run(SpeedtestTrigger::Manual).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SpeedtestNoNode);
    }

    #[tokio::test]
    async fn speedtest_releases_running_flag_after_error() {
        let api = Arc::new(FakeApi::new(vec![]));
        let (e, _h) = engine(api);
        assert!(!e.is_running());
        let _ = e.run(SpeedtestTrigger::Manual).await;
        assert!(!e.is_running(), "失败后必须释放，否则永远无法再次测速");
    }

    #[tokio::test]
    async fn auto_switch_picks_faster_node() {
        let api = Arc::new(
            FakeApi::new(vec![grp("Proxy", "slow", &["slow", "fast"])]).with_delays([
                ("slow".to_string(), Ok(500)),
                ("fast".to_string(), Ok(50)),
            ]),
        );
        let (e, hist) = engine(api.clone());
        let policy = crate::domain::model::SwitchPolicy::default();
        let recs = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();

        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].to_node, "fast");
        assert_eq!(recs[0].reason, SwitchReason::Best);
        assert_eq!(recs[0].to_delay, Some(50));
        assert_eq!(recs[0].from_delay, Some(500));

        // 确实调用了 PUT
        let sel = api.selected.lock().clone();
        assert_eq!(sel, vec![("Proxy".to_string(), "fast".to_string())]);
        // 历史已落盘
        assert_eq!(hist.recent(10).len(), 1);
    }

    #[tokio::test]
    async fn auto_switch_skips_when_kind_not_selector() {
        let mut fake = FakeApi::new(vec![grp("Proxy", "a", &["a", "b"])])
            .with_delays([("a".to_string(), Ok(900)), ("b".to_string(), Ok(10))]);
        // 模拟 API 返回 YAML 的小写 select（现实中不会，但用来验证过滤）
        fake.kind = "select".to_string();
        let api: Arc<dyn SelectorApi> = Arc::new(fake);
        let (e, _h) = engine(api);
        let policy = crate::domain::model::SwitchPolicy::default();
        let recs = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();
        assert!(recs.is_empty(), "kind != Selector 必须跳过，即使延迟差很大");
    }

    #[tokio::test]
    async fn auto_switch_respects_disabled_policy() {
        let api = Arc::new(
            FakeApi::new(vec![grp("Proxy", "a", &["a", "b"])]).with_delays([
                ("a".to_string(), Ok(900)),
                ("b".to_string(), Ok(10)),
            ]),
        );
        let (e, _h) = engine(api);
        let mut policy = crate::domain::model::SwitchPolicy::default();
        policy.enabled = false;
        let recs = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();
        assert!(recs.is_empty());
    }

    #[tokio::test]
    async fn auto_switch_within_tolerance_holds() {
        let api = Arc::new(
            FakeApi::new(vec![grp("Proxy", "a", &["a", "b"])]).with_delays([
                ("a".to_string(), Ok(100)),
                ("b".to_string(), Ok(90)),
            ]),
        );
        let (e, _h) = engine(api);
        let policy = crate::domain::model::SwitchPolicy::default(); // tolerance 30
        let recs = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();
        assert!(recs.is_empty(), "只快 10ms 不该切");
    }

    #[tokio::test]
    async fn manual_override_suppresses_auto_switch() {
        let api = Arc::new(
            FakeApi::new(vec![grp("Proxy", "a", &["a", "b"])]).with_delays([
                ("a".to_string(), Ok(900)),
                ("b".to_string(), Ok(10)),
            ]),
        );
        let (e, _h) = engine(api);
        e.note_manual_switch("Proxy", 60);
        let policy = crate::domain::model::SwitchPolicy::default();
        let recs = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();
        assert!(recs.is_empty(), "手动覆盖期内不得自动切换");
    }

    #[tokio::test]
    async fn failed_current_node_is_rescued() {
        let api = Arc::new(
            FakeApi::new(vec![grp("Proxy", "a", &["a", "b"])]).with_delays([
                ("a".to_string(), Err(ApiError::Timeout)),
                ("b".to_string(), Ok(80)),
            ]),
        );
        let (e, _h) = engine(api);
        let policy = crate::domain::model::SwitchPolicy::default(); // threshold 2

        // 第一次：失败 1 次，未达阈值
        let _ = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();
        // 第二次：达到 2 次 → 救急切换
        let recs = e.auto_switch(&policy, "http://x/204", 3000).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].reason, SwitchReason::CurrentFailed);
        assert_eq!(recs[0].to_node, "b");
    }

    #[tokio::test]
    async fn concurrent_run_is_rejected() {
        let api = Arc::new(FakeApi::new(vec![grp("Proxy", "a", &["a"])]));
        let (e, _h) = engine(api);
        // 手动占用 running 标记模拟进行中
        *e.running.lock() = Some("x".into());
        let err = e.run(SpeedtestTrigger::Manual).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SpeedtestRunning);
    }
}
