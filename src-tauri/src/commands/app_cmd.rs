//! 应用/设置/内核类 IPC 命令（C.6）。
//!
//! 命名 `<domain>_<action>` 蛇形，与契约逐字一致。

use std::sync::Arc;

use tauri::State;

use crate::commands::dto::*;
use crate::commands::state::AppHandleState;
use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{LogLevel, ProxyMode, Theme};

type R<T> = Result<T, IpcError>;

// ============================================================================
//  app
// ============================================================================

pub async fn app_info_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<AppInfoDto> {
    
    let settings = s.settings.get();
    Ok(AppInfoDto {
        version: crate::VERSION.to_string(),
        core_version: settings.core_version.clone(),
        data_dir: s.paths.roaming.to_string_lossy().to_string(),
        log_dir: s.paths.logs_dir().to_string_lossy().to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    })
}

#[tauri::command]
pub async fn app_info(state: State<'_, AppHandleState>) -> R<AppInfoDto> {
    app_info_inner(&state.0).await.map_err(IpcError::from)
}


/// 首次启动合规提示：用户接受后记录时间
pub async fn app_accept_compliance_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<OpResultDto> {
    
    s.settings
        .update(|x| x.compliance_accepted_at = Some(crate::util::time::now_secs()))
        ?;
    Ok(OpResultDto::ok("已记录"))
}

#[tauri::command]
pub async fn app_accept_compliance(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    app_accept_compliance_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn app_open_data_dir_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<OpResultDto> {
    
    let p = s.paths.roaming.to_string_lossy().to_string();
    open_in_explorer(&p)?;
    Ok(OpResultDto::ok("已打开数据目录"))
}

#[tauri::command]
pub async fn app_open_data_dir(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    app_open_data_dir_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn app_open_log_dir_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<OpResultDto> {
    
    let p = s.paths.logs_dir().to_string_lossy().to_string();
    open_in_explorer(&p)?;
    Ok(OpResultDto::ok("已打开日志目录"))
}

#[tauri::command]
pub async fn app_open_log_dir(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    app_open_log_dir_inner(&state.0).await.map_err(IpcError::from)
}


pub fn open_in_explorer(path: &str) -> AppResult<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?;
    Ok(())
}

// ============================================================================
//  settings
// ============================================================================

pub async fn settings_get_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<SettingsDto> {
    
    Ok(SettingsDto::from(&s.settings.get()))
}

#[tauri::command]
pub async fn settings_get(state: State<'_, AppHandleState>) -> R<SettingsDto> {
    settings_get_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn settings_update_inner(s: &Arc<crate::commands::state::AppState>, patch: SettingsPatch) -> AppResult<SettingsDto> {
    
    let updated = s
        .settings
        .update(|x| apply_patch(x, &patch))
        ?;

    // 设置变更后同步副作用
    sync_side_effects(s, &updated);

    Ok(SettingsDto::from(&updated))
}

#[tauri::command]
pub async fn settings_update(state: State<'_, AppHandleState>, patch: SettingsPatch) -> R<SettingsDto> {
    settings_update_inner(&state.0, patch).await.map_err(IpcError::from)
}


fn apply_patch(x: &mut crate::domain::model::AppSettings, p: &SettingsPatch) {
    if let Some(v) = &p.locale {
        x.locale = v.clone();
    }
    if let Some(v) = &p.theme {
        x.theme = match v.as_str() {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::System,
        };
    }
    if let Some(v) = p.auto_launch {
        x.auto_launch = v;
    }
    if let Some(v) = p.silent_start {
        x.silent_start = v;
    }
    if let Some(v) = p.start_core_on_launch {
        x.start_core_on_launch = v;
    }
    if let Some(v) = &p.proxy_mode {
        x.proxy_mode = match v.as_str() {
            "tun" => ProxyMode::Tun,
            "manual" => ProxyMode::Manual,
            // 兼容旧前端可能发的 "rule"/"global"：那是分流模式，不是代理方式，
            // 早期混用导致用户切"全局"时系统代理被意外关掉。这里只认代理方式。
            _ => ProxyMode::SystemProxy,
        };
    }
    if let Some(v) = &p.mihomo_mode {
        x.mihomo_mode = crate::domain::model::MihomoMode::parse(v);
    }
    if let Some(v) = p.api_port {
        x.api_port = v;
    }
    if let Some(v) = &p.core_version {
        x.core_version = if v.is_empty() { None } else { Some(v.clone()) };
    }
    if let Some(v) = &p.core_download_proxy {
        x.core_download_proxy = if v.trim().is_empty() {
            None
        } else {
            Some(v.trim().to_string())
        };
    }
    if let Some(v) = p.sub_default_interval_min {
        x.sub_default_interval_min = v;
    }
    if let Some(v) = p.sub_fetch_concurrency {
        x.sub_fetch_concurrency = v;
    }
    if let Some(v) = p.sub_fetch_timeout_ms {
        x.sub_fetch_timeout_ms = v;
    }
    if let Some(v) = &p.sub_fetch_proxy {
        x.sub_fetch_proxy = if v.trim().is_empty() {
            None
        } else {
            Some(v.trim().to_string())
        };
    }
    if let Some(v) = &p.sub_user_agent {
        x.sub_user_agent = v.clone();
    }
    if let Some(v) = p.speedtest_enabled {
        x.speedtest_enabled = v;
    }
    if let Some(v) = p.speedtest_interval_min {
        x.speedtest_interval_min = v;
    }
    if let Some(v) = &p.speedtest_probe_url {
        x.speedtest_probe_url = v.clone();
    }
    if let Some(v) = p.speedtest_timeout_ms {
        x.speedtest_timeout_ms = v;
    }
    if let Some(v) = p.speedtest_concurrency {
        x.speedtest_concurrency = v;
    }
    if let Some(pol) = &p.switch_policy {
        x.switch_policy.enabled = pol.enabled;
        x.switch_policy.tolerance_ms = pol.tolerance_ms;
        x.switch_policy.min_dwell_sec = pol.min_dwell_sec;
        x.switch_policy.failure_threshold = pol.failure_threshold;
        x.switch_policy.manual_override_min = pol.manual_override_min;
        x.switch_policy.exclude_keywords = pol.exclude_keywords.clone();
        x.switch_policy.prefer_region_priority = pol.prefer_region_priority.clone();
        // v3 失败转移
        x.switch_policy.on_failure_action = pol.on_failure_parsed();
        x.switch_policy.failover_enabled = pol.failover_enabled;
        x.switch_policy.retry_interval_sec = pol.retry_interval_sec;
        x.switch_policy.max_retries = pol.max_retries;
        x.switch_policy.dwell_after_switch_sec = pol.dwell_after_switch_sec;
        x.switch_policy.prefer_healthy_only = pol.prefer_healthy_only;
    }
    if let Some(v) = p.notify_on_switch {
        x.notify_on_switch = v;
    }
    if let Some(v) = p.notify_on_expire {
        x.notify_on_expire = v;
    }
    if let Some(v) = p.traffic_warn_percent {
        x.traffic_warn_percent = v;
    }
    if let Some(v) = &p.log_level {
        x.log_level = match v.as_str() {
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info,
        };
    }
    if let Some(v) = p.close_to_tray {
        x.close_to_tray = v;
    }
}

/// 设置生效时的副作用（自启动、测速参数、策略）
fn sync_side_effects(
    s: &Arc<crate::commands::state::AppState>,
    settings: &crate::domain::model::AppSettings,
) {
    // 测速参数热更新
    if let Some(engine) = s.speedtest.read().clone() {
        engine.update_config(crate::speedtest::ProbeConfig {
            probe_url: settings.speedtest_probe_url.clone(),
            timeout_ms: settings.speedtest_timeout_ms,
            concurrency: settings.speedtest_concurrency,
        });
    }
    s.emit(
        "settings:changed",
        serde_json::to_value(SettingsDto::from(settings)).unwrap_or_default(),
    );
}

// ============================================================================
//  core
// ============================================================================

pub async fn core_status_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<CoreStatusDto> {
    
    let st_ = s.kernel.state();
    let ports = s.kernel.actual_ports();
    let settings = s.settings.get();

    let (state_str, error) = match &st_ {
        crate::domain::model::CoreState::Stopped => ("stopped", None),
        crate::domain::model::CoreState::Starting => ("starting", None),
        crate::domain::model::CoreState::Running => ("running", None),
        crate::domain::model::CoreState::Stopping => ("stopping", None),
        crate::domain::model::CoreState::Failed(m) => ("failed", Some(m.clone())),
    };

    // ★ 修复「未安装」误判：以磁盘上的实际文件为准，而不是设置项。
    //
    // 早期直接返回 `settings.core_version`，用户没手动选过版本时它是 None，
    // 前端据此显示"内核未安装"——但内核其实早就装好了，重启后照样能跑。
    let installed_list = crate::kernel::fetch_core::list_installed(&s.paths);
    let installed_version = installed_list.first().map(|c| c.version.clone());
    let installed = !installed_list.is_empty();

    // version 优先用设置里指定的版本（用户可能装了多个版本手选其一），
    // 没指定就用磁盘上最新的那个 —— 这才是"当前实际会用到的版本"。
    let version = settings
        .core_version
        .clone()
        .filter(|v| installed_list.iter().any(|c| &c.version == v))
        .or_else(|| installed_version.clone());

    Ok(CoreStatusDto {
        state: state_str.to_string(),
        error,
        version,
        mixed_port: ports.map(|p| p.0),
        api_port: ports.map(|p| p.1),
        uptime_sec: s.core_uptime(),
        installed,
        installed_version,
    })
}

#[tauri::command]
pub async fn core_status(state: State<'_, AppHandleState>) -> R<CoreStatusDto> {
    core_status_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn core_start_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<CoreStatusDto> {
    
    crate::app::start_core(s).await?;
    core_status_inner(s).await
}

#[tauri::command]
pub async fn core_start(state: State<'_, AppHandleState>) -> R<CoreStatusDto> {
    core_start_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn core_stop_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<CoreStatusDto> {
    
    // 停止前先撤掉系统代理，否则断网
    if s.sysproxy.read_state().managed_by_us {
        let _ = s.sysproxy.disable();
    }
    s.kernel.stop().await?;
    s.detach_core();
    s.emit("core:state", serde_json::json!({"state": "stopped"}));
    core_status_inner(s).await
}

#[tauri::command]
pub async fn core_stop(state: State<'_, AppHandleState>) -> R<CoreStatusDto> {
    core_stop_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn core_restart_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<CoreStatusDto> {
    
    if s.sysproxy.read_state().managed_by_us {
        let _ = s.sysproxy.disable();
    }
    let _ = s.kernel.stop().await;
    s.detach_core();
    crate::app::start_core(s).await?;
    core_status_inner(s).await
}

#[tauri::command]
pub async fn core_restart(state: State<'_, AppHandleState>) -> R<CoreStatusDto> {
    core_restart_inner(&state.0).await.map_err(IpcError::from)
}


/// 列出已安装内核版本
pub async fn core_list_versions_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<Vec<InstalledCoreDto>> {
    
    let active = s.settings.get().core_version;
    Ok(crate::kernel::fetch_core::list_installed(&s.paths)
        .into_iter()
        .map(|c| InstalledCoreDto {
            is_active: active.as_deref() == Some(c.version.as_str()),
            version: c.version,
            size: c.size,
            installed_at: c.installed_at,
        })
        .collect())
}

#[tauri::command]
pub async fn core_list_versions(state: State<'_, AppHandleState>) -> R<Vec<InstalledCoreDto>> {
    core_list_versions_inner(&state.0).await.map_err(IpcError::from)
}


/// 下载并安装内核（异步，进度通过事件推送）
pub async fn core_install_inner(s: &Arc<crate::commands::state::AppState>, version: Option<String>) -> AppResult<OpResultDto> {
    
    let version = version
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::kernel::fetch_core::DEFAULT_CORE_VERSION.to_string());
    let settings = s.settings.get();

    let paths = Arc::clone(&s.paths);
    let proxy = settings.core_download_proxy.clone();
    let events = s.events.clone();
    let v = version.clone();

    // 后台执行，避免阻塞 IPC（下载可能几十秒）
    tokio::spawn(async move {
        let v_progress = v.clone();
        let events_progress = events.clone();
        let progress = move |done: u64, total: Option<u64>| {
            let _ = events_progress.send(crate::commands::state::AppEvent {
                kind: "core:downloadProgress".into(),
                payload: serde_json::json!({
                    "version": v_progress,
                    "downloaded": done,
                    "total": total,
                }),
                at: crate::util::time::now_secs(),
            });
        };
        match crate::kernel::fetch_core::download_and_install(
            &paths,
            &v,
            proxy.as_deref(),
            progress,
        )
        .await
        {
            Ok(core) => {
                let _ = events.send(crate::commands::state::AppEvent {
                    kind: "core:installed".into(),
                    payload: serde_json::json!({ "version": core.version }),
                    at: crate::util::time::now_secs(),
                });
            }
            Err(e) => {
                let _ = events.send(crate::commands::state::AppEvent {
                    kind: "core:installFailed".into(),
                    payload: serde_json::json!({
                        "code": e.code.as_str(),
                        "message": e.message,
                    }),
                    at: crate::util::time::now_secs(),
                });
            }
        }
    });

    Ok(OpResultDto::ok(format!("已开始下载内核 {version}")))
}

#[tauri::command]
pub async fn core_install(state: State<'_, AppHandleState>, version: Option<String>) -> R<OpResultDto> {
    core_install_inner(&state.0, version).await.map_err(IpcError::from)
}


// ============================================================================
//  sysproxy
// ============================================================================

pub async fn sysproxy_get_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<RuntimeDto> {
    
    Ok(runtime_dto(s))
}

#[tauri::command]
pub async fn sysproxy_get(state: State<'_, AppHandleState>) -> R<RuntimeDto> {
    sysproxy_get_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn sysproxy_enable_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<RuntimeDto> {
    
    let ports = s
        .kernel
        .actual_ports()
        .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))?;
    s.sysproxy.enable(ports.0)?;
    s.emit(
        "sysproxy:changed",
        serde_json::json!({"enabled": true, "port": ports.0}),
    );
    Ok(runtime_dto(s))
}

#[tauri::command]
pub async fn sysproxy_enable(state: State<'_, AppHandleState>) -> R<RuntimeDto> {
    sysproxy_enable_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn sysproxy_disable_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<RuntimeDto> {
    
    s.sysproxy.disable()?;
    s.emit("sysproxy:changed", serde_json::json!({"enabled": false}));
    Ok(runtime_dto(s))
}

#[tauri::command]
pub async fn sysproxy_disable(state: State<'_, AppHandleState>) -> R<RuntimeDto> {
    sysproxy_disable_inner(&state.0).await.map_err(IpcError::from)
}


pub fn runtime_dto(s: &Arc<crate::commands::state::AppState>) -> RuntimeDto {
    let sy = s.sysproxy.read_state();
    let core = s.kernel.state();
    let ports = s.kernel.actual_ports();
    let (state_str, error) = match &core {
        crate::domain::model::CoreState::Stopped => ("stopped", None),
        crate::domain::model::CoreState::Starting => ("starting", None),
        crate::domain::model::CoreState::Running => ("running", None),
        crate::domain::model::CoreState::Stopping => ("stopping", None),
        crate::domain::model::CoreState::Failed(m) => ("failed", Some(m.clone())),
    };
    let settings = s.settings.get();
    let installed_list = crate::kernel::fetch_core::list_installed(&s.paths);
    let installed_version = installed_list.first().map(|c| c.version.clone());
    let version = settings
        .core_version
        .clone()
        .filter(|v| installed_list.iter().any(|c| &c.version == v))
        .or_else(|| installed_version.clone());
    RuntimeDto {
        core: CoreStatusDto {
            state: state_str.to_string(),
            error,
            version,
            mixed_port: ports.map(|p| p.0),
            api_port: ports.map(|p| p.1),
            uptime_sec: s.core_uptime(),
            installed: !installed_list.is_empty(),
            installed_version,
        },
        sysproxy_enabled: sy.enabled,
        sysproxy_managed: sy.managed_by_us,
        sysproxy_server: sy.server,
        active_subscription: None,
        traffic_up: 0,
        traffic_down: 0,
        up_rate: 0,
        down_rate: 0,
    }
}

// ============================================================================
//  runtime
// ============================================================================

pub async fn runtime_get_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<RuntimeDto> {
    
    Ok(runtime_dto(s))
}

#[tauri::command]
pub async fn runtime_get(state: State<'_, AppHandleState>) -> R<RuntimeDto> {
    runtime_get_inner(&state.0).await.map_err(IpcError::from)
}


/// 打开窗口（托盘菜单用）
///
/// 窗口类操作依赖 Tauri 的 AppHandle，**不提取 `_inner`**：
/// daemon 进程里没有窗口，这些命令只存在于 Tauri 侧。
#[tauri::command]
pub async fn runtime_show_window(app: tauri::AppHandle) -> R<OpResultDto> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    Ok(OpResultDto::ok("已显示"))
}

/// 窗口控制（自绘标题栏需要）
#[tauri::command]
pub async fn window_minimize(app: tauri::AppHandle) -> R<OpResultDto> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.minimize();
    }
    Ok(OpResultDto::ok("ok"))
}

#[tauri::command]
pub async fn window_toggle_maximize(app: tauri::AppHandle) -> R<OpResultDto> {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let max = w.is_maximized().unwrap_or(false);
        if max {
            let _ = w.unmaximize();
        } else {
            let _ = w.maximize();
        }
    }
    Ok(OpResultDto::ok("ok"))
}

#[tauri::command]
pub async fn window_close(app: tauri::AppHandle, state: State<'_, AppHandleState>) -> R<OpResultDto> {
    use tauri::Manager;
    let s = &state.0;
    let to_tray = s.settings.get().close_to_tray;
    if let Some(w) = app.get_webview_window("main") {
        if to_tray {
            let _ = w.hide();
        } else {
            // 真退出：先清理
            if s.sysproxy.read_state().managed_by_us {
                let _ = s.sysproxy.disable();
            }
            let _ = s.kernel.stop().await;
            app.exit(0);
        }
    }
    Ok(OpResultDto::ok("ok"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::AppSettings;

    #[test]
    fn patch_applies_theme() {
        let mut s = AppSettings::default();
        let mut p = SettingsPatch::default();
        p.theme = Some("dark".into());
        apply_patch(&mut s, &p);
        assert_eq!(s.theme, Theme::Dark);
    }

    #[test]
    fn patch_empty_proxy_becomes_none() {
        let mut s = AppSettings::default();
        s.sub_fetch_proxy = Some("http://127.0.0.1:1886".into());
        let mut p = SettingsPatch::default();
        p.sub_fetch_proxy = Some("   ".into());
        apply_patch(&mut s, &p);
        assert_eq!(s.sub_fetch_proxy, None, "清空应移除代理而非留空串");
    }

    #[test]
    fn patch_clamps_via_store_sanitize() {
        // apply_patch 只赋值；sanitize 由 SettingsStore::update 负责。
        // 这里验证 sanitize 本身能纠正越界值。
        let mut s = AppSettings::default();
        let mut p = SettingsPatch::default();
        p.sub_default_interval_min = Some(1);
        p.speedtest_interval_min = Some(1);
        apply_patch(&mut s, &p);
        s.sanitize();
        assert_eq!(s.sub_default_interval_min, 15);
        assert_eq!(s.speedtest_interval_min, 5);
    }

    #[test]
    fn patch_updates_policy() {
        let mut s = AppSettings::default();
        let mut p = SettingsPatch::default();
        p.switch_policy = Some(PolicyDto {
            enabled: false,
            tolerance_ms: 50,
            min_dwell_sec: 600,
            failure_threshold: 3,
            manual_override_min: 30,
            exclude_keywords: vec!["测试".into()],
            prefer_region_priority: vec!["HK".into()],
            on_failure_action: "retry".into(),
            failover_enabled: false,
            retry_interval_sec: 10,
            max_retries: 5,
            dwell_after_switch_sec: 120,
            prefer_healthy_only: false,
        });
        apply_patch(&mut s, &p);
        assert!(!s.switch_policy.enabled);
        assert_eq!(s.switch_policy.tolerance_ms, 50);
        assert_eq!(s.switch_policy.exclude_keywords, vec!["测试"]);
        // v3 字段
        assert_eq!(
            s.switch_policy.on_failure_action,
            crate::domain::model::OnFailureAction::Retry
        );
        assert!(!s.switch_policy.failover_enabled);
        assert_eq!(s.switch_policy.retry_interval_sec, 10);
        assert_eq!(s.switch_policy.max_retries, 5);
        assert_eq!(s.switch_policy.dwell_after_switch_sec, 120);
        assert!(!s.switch_policy.prefer_healthy_only);
    }

    #[test]
    fn patch_core_version_empty_clears() {
        let mut s = AppSettings::default();
        s.core_version = Some("v1.19.33".into());
        let mut p = SettingsPatch::default();
        p.core_version = Some(String::new());
        apply_patch(&mut s, &p);
        assert_eq!(s.core_version, None, "空串表示使用默认版本");
    }

    #[test]
    fn patch_log_level_maps() {
        for (input, expect) in [
            ("error", LogLevel::Error),
            ("warn", LogLevel::Warn),
            ("debug", LogLevel::Debug),
            ("info", LogLevel::Info),
            ("unknown", LogLevel::Info),
        ] {
            let mut s = AppSettings::default();
            let mut p = SettingsPatch::default();
            p.log_level = Some(input.into());
            apply_patch(&mut s, &p);
            assert_eq!(s.log_level, expect, "输入 {input}");
        }
    }

    #[test]
    fn patch_proxy_mode_maps() {
        for (input, expect) in [
            ("tun", ProxyMode::Tun),
            ("manual", ProxyMode::Manual),
            ("system_proxy", ProxyMode::SystemProxy),
            ("bogus", ProxyMode::SystemProxy),
        ] {
            let mut s = AppSettings::default();
            let mut p = SettingsPatch::default();
            p.proxy_mode = Some(input.into());
            apply_patch(&mut s, &p);
            assert_eq!(s.proxy_mode, expect);
        }
    }

    #[test]
    fn patch_mihomo_mode_maps() {
        use crate::domain::model::MihomoMode;
        for (input, expect) in [
            ("rule", MihomoMode::Rule),
            ("global", MihomoMode::Global),
            ("direct", MihomoMode::Direct),
            ("bogus", MihomoMode::Rule),
        ] {
            let mut s = AppSettings::default();
            let mut p = SettingsPatch::default();
            p.mihomo_mode = Some(input.into());
            apply_patch(&mut s, &p);
            assert_eq!(s.mihomo_mode, expect, "输入 {input}");
        }
    }

    #[test]
    fn mihomo_mode_does_not_touch_proxy_mode() {
        // 关键回归：切「全局」不得顺手改掉代理方式（早期混用导致系统代理被关）
        use crate::domain::model::MihomoMode;
        let mut s = AppSettings::default();
        s.proxy_mode = ProxyMode::Tun;

        let mut p = SettingsPatch::default();
        p.mihomo_mode = Some("global".into());
        apply_patch(&mut s, &p);

        assert_eq!(s.mihomo_mode, MihomoMode::Global);
        assert_eq!(s.proxy_mode, ProxyMode::Tun, "代理方式不得被分流模式改动");
    }
}
