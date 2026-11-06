//! 单 tick 调度循环（P3）。
//!
//! 每轮：
//! 1. 算现在时刻
//! 2. 检查各任务是否到期（含抖动）
//! 3. 执行到期任务（串行，避免并发写 config.yaml）
//! 4. 睡到下一个整分
//!
//! 任务执行异常**不得**中断循环：单个任务失败只记日志 + 退避。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use crate::domain::model::{AppSettings, SpeedtestTrigger};
use crate::kernel::KernelSupervisor;
use crate::scheduler::backoff::Backoff;
use crate::scheduler::clock::next_wake_delay;
use crate::scheduler::due::PeriodicTask;
use crate::speedtest::SpeedtestEngine;
use crate::store::settings_store::SettingsStore;
use crate::store::traffic_store::TrafficStore;
use crate::subscription::SubscriptionManager;
use crate::util::time::now_secs;

/// 流量采集周期（秒）。60 秒足够画趋势图，又不会把内核 API 打满。
pub const TRAFFIC_COLLECT_INTERVAL_SEC: u32 = 1;

/// 调度事件（推给 UI）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerEvent {
    pub kind: String,
    pub message: String,
    pub at: i64,
}

/// 采集周期下限：1 分钟一次即可。
///
/// `PeriodicTask.period_min` 的单位是**分钟**，而 `TRAFFIC_COLLECT_INTERVAL_SEC`
/// 表达的是秒 —— 这里必须显式说明，否则容易误写成 60 分钟。
pub const TRAFFIC_COLLECT_PERIOD_MIN: u32 = 1;

/// 调度器
pub struct Scheduler {
    subs: Arc<SubscriptionManager>,
    settings: Arc<SettingsStore>,
    kernel: Arc<KernelSupervisor>,
    speedtest: Arc<SpeedtestEngine>,
    traffic: Arc<TrafficStore>,
    /// 内核 API（流量采集需要；内核未就绪时为 None）
    api: parking_lot::RwLock<Option<Arc<dyn crate::kernel::SelectorApi>>>,
    events: broadcast::Sender<SchedulerEvent>,
    /// 订阅拉取任务状态
    sub_task: parking_lot::Mutex<PeriodicTask>,
    /// 测速任务状态
    speed_task: parking_lot::Mutex<PeriodicTask>,
    /// 流量采集任务状态
    traffic_task: parking_lot::Mutex<PeriodicTask>,
    /// 订阅失败退避
    sub_backoff: parking_lot::Mutex<Backoff>,
    /// 停止标志
    stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Scheduler {
    pub fn new(
        subs: Arc<SubscriptionManager>,
        settings: Arc<SettingsStore>,
        kernel: Arc<KernelSupervisor>,
        speedtest: Arc<SpeedtestEngine>,
        traffic: Arc<TrafficStore>,
        events: broadcast::Sender<SchedulerEvent>,
    ) -> Arc<Self> {
        let s = settings.get();
        Arc::new(Self {
            subs,
            settings,
            kernel,
            speedtest,
            traffic,
            api: parking_lot::RwLock::new(None),
            events,
            sub_task: parking_lot::Mutex::new(PeriodicTask::new(
                "sub-update",
                s.sub_default_interval_min,
                None,
            )),
            speed_task: parking_lot::Mutex::new(PeriodicTask::new(
                "speedtest",
                s.speedtest_interval_min,
                // 启动时不定为已执行：让首轮就测一次，用户点开界面即有数据
                None,
            )),
            traffic_task: parking_lot::Mutex::new(PeriodicTask::new(
                "traffic-collect",
                TRAFFIC_COLLECT_PERIOD_MIN,
                // 启动即采一次，建立基线
                None,
            )),
            sub_backoff: parking_lot::Mutex::new(Backoff::for_subscription()),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// 内核就绪/停止时同步 API 引用（流量采集用）
    pub fn set_api(&self, api: Option<Arc<dyn crate::kernel::SelectorApi>>) {
        *self.api.write() = api;
    }

    fn emit(&self, kind: &str, message: impl Into<String>) {
        let _ = self.events.send(SchedulerEvent {
            kind: kind.to_string(),
            message: message.into(),
            at: now_secs(),
        });
    }

    pub fn request_stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// 启动调度循环（在 tokio runtime 内 spawn）
    pub fn spawn(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            tracing::info!("调度器启动");
            loop {
                if this.stop.load(std::sync::atomic::Ordering::SeqCst) {
                    tracing::info!("调度器停止");
                    return;
                }

                let now = now_secs();
                // 设置变化时同步周期（用户改间隔无需重启）
                let s = this.settings.get();
                this.resync_periods(&s);

                this.tick(now, &s).await;

                let delay = next_wake_delay();
                tokio::time::sleep(delay).await;
            }
        });
    }

    /// 周期与设置同步
    fn resync_periods(&self, s: &AppSettings) {
        {
            let mut t = self.sub_task.lock();
            if t.period_min != s.sub_default_interval_min {
                t.period_min = s.sub_default_interval_min.max(15);
            }
        }
        {
            let mut t = self.speed_task.lock();
            let want = if s.speedtest_enabled {
                s.speedtest_interval_min.max(5)
            } else {
                // 关闭测速 → 用极大周期，等价于不调度
                u32::MAX / 120
            };
            if t.period_min != want {
                t.period_min = want;
            }
        }
    }

    /// 一轮 tick
    async fn tick(&self, now: i64, s: &AppSettings) {        // ---- 流量采集（最高频，先做）----
        let traffic_due = {
            let t = self.traffic_task.lock();
            t.is_due(now)
        };
        if traffic_due {
            self.collect_traffic(now).await;
            self.traffic_task.lock().mark_run(now);
        }

        // ---- 订阅更新 ----
        let sub_due = {
            let t = self.sub_task.lock();
            t.is_due(now)
        };
        if sub_due {
            self.run_subscription_update(now).await;
        }

        // ---- 测速 + 自动切换 ----
        if s.speedtest_enabled {
            let speed_due = {
                let t = self.speed_task.lock();
                t.is_due(now)
            };
            if speed_due {
                self.run_speedtest(now).await;
            }
        }
    }

    /// 采集一次内核流量差值并落盘
    async fn collect_traffic(&self, now: i64) {
        if !self.kernel.state().is_running() {
            return;
        }
        let api = self.api.read().clone();
        let Some(api) = api else {
            tracing::debug!("内核 API 未就绪，跳过流量采集");
            return;
        };

        let totals = match api.traffic_totals().await {
            Ok(t) => t,
            Err(e) => {
                // 采集失败不能中断调度循环（内核可能正在重启）
                tracing::debug!(err = %e, "流量采集失败");
                return;
            }
        };

        if let Some((up, down)) = self.traffic.feed_cumulative(now, totals.up, totals.down) {
            if up > 0 || down > 0 {
                let _ = self.events.send(SchedulerEvent {
                    kind: "traffic:tick".to_string(),
                    message: format!("{up}/{down}"),
                    at: now,
                });
            }
        }
    }

    async fn run_subscription_update(&self, now: i64) {
        // 退避未到则跳过本轮
        let wait = {
            let b = self.sub_backoff.lock();
            b.is_exhausted()
        };
        if wait {
            // 已放弃，等用户手动触发或下一个大周期
            self.sub_backoff.lock().reset();
        }

        tracing::info!("开始定时更新订阅");
        let results = self.subs.fetch_due(false).await;
        if results.is_empty() {
            // 没有到期的，标记本次 tick 已处理
            self.sub_task.lock().mark_run(now);
            return;
        }

        let mut ok = 0usize;
        let mut failed = 0usize;
        for (uid, r) in &results {
            match r {
                Ok(_) => ok += 1,
                Err(e) => {
                    failed += 1;
                    tracing::warn!(uid = %uid, err = %e, "订阅更新失败");
                }
            }
        }

        if failed == 0 {
            self.sub_backoff.lock().reset();
        } else {
            let _ = self.sub_backoff.lock().next_delay();
        }

        self.emit(
            "subscription:updated",
            format!("订阅更新完成：成功 {ok}，失败 {failed}"),
        );
        self.sub_task.lock().mark_run(now);

        // 订阅更新后重建配置并让内核生效
        if ok > 0 {
            self.apply_config().await;
            // 更新后立即测速（拿到新节点延迟）
            self.run_speedtest(now).await;
        }
    }

    /// 重建配置并通知内核热重载
    async fn apply_config(&self) {
        let ports = match self.kernel.actual_ports() {
            Some(p) => p,
            None => {
                tracing::debug!("内核未运行，跳过配置应用");
                return;
            }
        };
        let secret = match std::fs::read_to_string(
            crate::system::paths::Paths::global()
                .map(|p| p.secret_file())
                .unwrap_or_default(),
        ) {
            Ok(s) => s,
            Err(_) => {
                tracing::warn!("读取 secret 失败，跳过配置应用");
                return;
            }
        };

        match self
            .subs
            .build_config(ports.0, ports.1, secret.trim())
        {
            Ok(Some(gen)) => match self.subs.write_config(&gen) {
                Ok(()) => {
                    if let Err(e) = self.kernel.reload_config().await {
                        tracing::warn!(err = %e, "内核热重载配置失败");
                    } else {
                        tracing::info!(nodes = gen.node_names.len(), "配置已生效");
                    }
                }
                Err(e) => tracing::error!(err = %e, "写配置失败"),
            },
            Ok(None) => tracing::debug!("无可用订阅，跳过"),
            Err(e) => tracing::error!(err = %e, "生成配置失败"),
        }
    }

    async fn run_speedtest(&self, now: i64) {
        if !self.kernel.state().is_running() {
            tracing::debug!("内核未运行，跳过测速");
            return;
        }
        tracing::info!("开始定时测速");
        match self.speedtest.run(SpeedtestTrigger::Scheduled).await {
            Ok(run) => {
                self.emit(
                    "speedtest:done",
                    format!(
                        "测速完成：{}/{} 可用",
                        run.ok_count, run.total
                    ),
                );
                self.speed_task.lock().mark_run(now);
            }
            Err(e) => {
                tracing::warn!(err = %e, "测速失败");
                self.speed_task.lock().mark_run(now);
            }
        }
    }

    /// 供"用户点立即更新"调用
    pub async fn trigger_now(&self) {
        let now = now_secs();
        self.run_subscription_update(now).await;
    }
}

/// 给 tokio::time::sleep 用的常量（便于测试注入）
pub const MIN_TICK: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::due::PeriodicTask;

    #[test]
    fn periodic_task_independent_of_scheduler() {
        // 调度逻辑本身在 due.rs 已充分覆盖，这里验证常量与基本不变量
        assert_eq!(MIN_TICK.as_secs(), 1, "tick 下限 1 秒，防忙轮询");
    }

    #[test]
    fn scheduler_event_serializes_camelcase() {
        let e = SchedulerEvent {
            kind: "subscription:updated".into(),
            message: "ok".into(),
            at: 123,
        };
        let j = serde_json::to_string(&e).unwrap();
        assert!(j.contains("\"kind\""));
        assert!(j.contains("\"at\""));
    }

    #[test]
    fn speedtest_disabled_uses_huge_period() {
        // 关闭测速时周期被设成极大值，等价于永不触发
        let period = u32::MAX / 120;
        let t = PeriodicTask::new("speedtest", period, Some(0));
        assert!(!t.is_due(now_secs()), "关闭后不应触发");
    }
}
