//! 应用全局状态（Tauri `State` 注入）。
//!
//! 所有字段都是 `Arc`，命令层拿到的只是引用拷贝，不复制数据。

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::kernel::{HttpKernelApi, KernelSupervisor, SelectorApi};
use crate::scheduler::Scheduler;
use crate::speedtest::SpeedtestEngine;
use crate::store::settings_store::{SettingsStore, SubscriptionStore};
use crate::store::switch_history::SwitchHistoryStore;
use crate::subscription::SubscriptionManager;
use crate::system::paths::Paths;
use crate::system::sysproxy::SysProxyManager;

/// 事件总线载荷：统一为 `{ kind, payload }` 便于前端分发
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEvent {
    /// 事件名，形如 `core:state`
    pub kind: String,
    pub payload: serde_json::Value,
    pub at: i64,
}

pub struct AppState {
    pub paths: Arc<Paths>,
    pub settings: Arc<SettingsStore>,
    pub subs_store: Arc<SubscriptionStore>,
    pub subs: Arc<SubscriptionManager>,
    pub history: Arc<SwitchHistoryStore>,
    pub sysproxy: Arc<SysProxyManager>,
    pub kernel: Arc<KernelSupervisor>,
    /// 内核 API（内核未运行时为 None）
    pub api: parking_lot::RwLock<Option<Arc<dyn SelectorApi>>>,
    pub speedtest: parking_lot::RwLock<Option<Arc<SpeedtestEngine>>>,
    pub scheduler: parking_lot::RwLock<Option<Arc<Scheduler>>>,
    /// 全局事件出口
    pub events: broadcast::Sender<AppEvent>,
    /// 内核启动时刻
    pub core_started_at: parking_lot::Mutex<i64>,
    /// 当前生效的 secret
    pub secret: parking_lot::RwLock<String>,
}

impl AppState {
    pub fn new(paths: Arc<Paths>) -> AppResult<Arc<Self>> {
        let settings = Arc::new(SettingsStore::load(&paths)?);
        let subs_store = Arc::new(SubscriptionStore::load(&paths)?);
        let history = Arc::new(SwitchHistoryStore::load(&paths)?);
        let sysproxy = Arc::new(SysProxyManager::new(&paths));
        let subs = Arc::new(SubscriptionManager::new(
            Arc::clone(&paths),
            Arc::clone(&subs_store),
            Arc::clone(&settings),
        ));

        let (core_tx, _) = broadcast::channel(64);
        let kernel = KernelSupervisor::new(core_tx);
        let (events, _) = broadcast::channel(256);

        Ok(Arc::new(Self {
            paths,
            settings,
            subs_store,
            subs,
            history,
            sysproxy,
            kernel,
            api: parking_lot::RwLock::new(None),
            speedtest: parking_lot::RwLock::new(None),
            scheduler: parking_lot::RwLock::new(None),
            events,
            core_started_at: parking_lot::Mutex::new(0),
            secret: parking_lot::RwLock::new(String::new()),
        }))
    }

    /// 取当前内核 API，未就绪时报 `E_CORE_NOT_RUNNING`
    pub fn require_api(&self) -> AppResult<Arc<dyn SelectorApi>> {
        self.api
            .read()
            .clone()
            .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))
    }

    pub fn require_speedtest(&self) -> AppResult<Arc<SpeedtestEngine>> {
        self.speedtest
            .read()
            .clone()
            .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))
    }

    /// 内核就绪后装配 API 与测速引擎
    pub fn attach_core(&self, api_port: u16, secret: &str) -> AppResult<()> {
        let http = HttpKernelApi::new(api_port, secret.to_string())?;
        let api: Arc<dyn SelectorApi> = Arc::new(http);
        *self.api.write() = Some(Arc::clone(&api));
        *self.secret.write() = secret.to_string();

        let (st_tx, _) = broadcast::channel(256);
        let (sw_tx, _) = broadcast::channel(256);
        let engine = SpeedtestEngine::new(
            api,
            Arc::clone(&self.history),
            st_tx,
            sw_tx,
        );
        let s = self.settings.get();
        engine.update_config(crate::speedtest::ProbeConfig {
            probe_url: s.speedtest_probe_url.clone(),
            timeout_ms: s.speedtest_timeout_ms,
            concurrency: s.speedtest_concurrency,
        });
        *self.speedtest.write() = Some(engine);
        *self.core_started_at.lock() = crate::util::time::now_secs();
        tracing::info!(api_port, "内核 API 已装配");
        Ok(())
    }

    /// 内核停止后卸载（避免命令层拿到失效 API 一直超时）
    pub fn detach_core(&self) {
        *self.api.write() = None;
        *self.speedtest.write() = None;
        *self.core_started_at.lock() = 0;
    }

    pub fn emit(&self, kind: &str, payload: serde_json::Value) {
        let _ = self.events.send(AppEvent {
            kind: kind.to_string(),
            payload,
            at: crate::util::time::now_secs(),
        });
    }

    /// 内核运行时长（秒）
    pub fn core_uptime(&self) -> i64 {
        let start = *self.core_started_at.lock();
        if start == 0 {
            0
        } else {
            (crate::util::time::now_secs() - start).max(0)
        }
    }
}

/// Tauri 管理的状态包装
pub struct AppHandleState(pub Arc<AppState>);
