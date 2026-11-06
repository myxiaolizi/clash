//! 应用启动编排：内核拉起、系统代理恢复、调度器启动、托盘与窗口。

use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::commands::state::{AppEvent, AppState};
use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::ProxyMode;
use crate::kernel::CoreLaunchSpec;

/// 内核启动全过程（首次安装 → 分配端口 → 写配置 → 启动 → 校验 → 装 API）
pub async fn start_core(s: &Arc<AppState>) -> AppResult<()> {
    let settings = s.settings.get();

    // ---- 1. 确认内核存在 ----
    let version = settings
        .core_version
        .clone()
        .unwrap_or_else(|| crate::kernel::fetch_core::DEFAULT_CORE_VERSION.to_string());

    let exe = s.paths.core_binary(&version);
    if !exe.exists() {
        // 自动下载一次（首次启动体验）
        tracing::info!(version = %version, "内核不存在，开始自动下载");
        s.emit("core:state", serde_json::json!({"state": "starting", "message": "正在下载内核"}));
        crate::kernel::fetch_core::download_and_install(
            &s.paths,
            &version,
            settings.core_download_proxy.as_deref(),
            |_, _| {},
        )
        .await?;
    }

    // ---- 2. 分配端口 ----
    let (mixed_port, api_port) = if settings.api_port > 0 {
        // 用户指定 API 端口：仍需确认可用
        if crate::kernel::ports::is_port_free(settings.api_port) {
            let m = crate::kernel::ports::find_free_port(
                crate::kernel::ports::MIXED_PORT_START,
                100,
            )?;
            (m, settings.api_port)
        } else {
            crate::kernel::ports::allocate_port_pair()?
        }
    } else {
        crate::kernel::ports::allocate_port_pair()?
    };

    // ---- 3. secret（持久化，避免每次重启都变） ----
    let secret = load_or_create_secret(s)?;

    // ---- 4. 生成配置 ----
    let gen = s
        .subs
        .build_config(mixed_port, api_port, &secret)?;

    match gen {
        Some(g) => {
            s.subs.write_config(&g)?;
            tracing::info!(nodes = g.node_names.len(), "配置已生成");
        }
        None => {
            // 无订阅也必须有配置，否则内核起不来（用户首次打开软件）
            crate::subscription::generator::generate(
                &crate::subscription::generator::GenerateInput {
                    subscriptions: vec![],
                    mixed_port,
                    api_port,
                    secret: secret.clone(),
                    main_group: crate::subscription::generator::MAIN_GROUP_DEFAULT.to_string(),
                    log_level: settings.log_level.to_mihomo(),
                    tun_enabled: matches!(settings.proxy_mode, ProxyMode::Tun),
                    mihomo_mode: settings.mihomo_mode,
                    user_rules: Vec::new(),
                },
            )
            .map(|g| s.subs.write_config(&g))
            .unwrap_or_else(|_| write_placeholder_config(s, mixed_port, api_port, &secret))?;
        }
    }

    // ---- 5. 启动内核 ----
    let spec = CoreLaunchSpec::from_paths(&s.paths, &version, api_port, mixed_port, secret.clone());
    s.kernel.start(spec).await?;

    // ---- 6. 校验端口未漂移 ----
    if let Err(e) = s.kernel.verify_ports().await {
        tracing::warn!(err = %e, "端口校验失败，尝试重启内核");
        let _ = s.kernel.stop().await;
        return Err(e);
    }

    // ---- 7. 装配 API 与测速引擎 ----
    s.attach_core(api_port, &secret)?;

    // ---- 8. 系统代理（按设置） ----
    if matches!(settings.proxy_mode, ProxyMode::SystemProxy) {
        match s.sysproxy.enable(mixed_port) {
            Ok(_) => tracing::info!(port = mixed_port, "系统代理已开启"),
            Err(e) => {
                // 系统代理失败不应导致内核启动失败（用户可能只是没权限）
                tracing::warn!(err = %e, "系统代理开启失败，内核仍在运行");
                s.emit(
                    "sysproxy:failed",
                    serde_json::json!({"code": e.code.as_str(), "message": e.message}),
                );
            }
        }
    }

    s.emit(
        "core:state",
        serde_json::json!({
            "state": "running",
            "mixedPort": mixed_port,
            "apiPort": api_port,
        }),
    );
    tracing::info!(mixed = mixed_port, api = api_port, "内核启动流程完成");
    Ok(())
}

/// 写一个仅含最小字段的占位配置（无订阅时保证内核能启动）
///
/// **关键约束**：mihomo 拒绝 `proxies: []` 的空 select 组：
/// `Parse config error: proxy group[0]: Proxy: 'use' or 'proxies' missing`
/// 它把空序列当成"字段缺失"。因此在没有节点时**根本不写 proxy-groups**，
/// 只保留 DIRECT/REJECT 内置策略，让内核能正常起来等用户添加订阅。
fn write_placeholder_config(
    s: &Arc<AppState>,
    mixed_port: u16,
    api_port: u16,
    secret: &str,
) -> AppResult<()> {
    // 占位配置同样要遵守用户选的分流模式：否则用户在"全局"模式下
    // 还没加订阅时看到的是规则模式行为，切换态与实际不符。
    let mode = s.settings.get().mihomo_mode.as_str();
    let yaml = format!(
        "mixed-port: {mixed_port}\n\
         allow-lan: false\n\
         bind-address: 127.0.0.1\n\
         mode: {mode}\n\
         log-level: info\n\
         find-process-mode: strict\n\
         ipv6: true\n\
         external-controller: 127.0.0.1:{api_port}\n\
         secret: {secret}\n\
         profile:\n  \
           store-selected: true\n  \
           store-fake-ip: false\n\
         dns:\n  \
           enable: false\n\
         proxies: []\n\
         rules:\n  \
           - MATCH,DIRECT\n"
    );
    // 生成后仍要过断言，保持一致性（只校验"不含自动类型组"，
    // 空配置本来就没有组）
    crate::subscription::generator::assert_no_auto_groups(&yaml)?;
    crate::system::paths::atomic_write(&s.paths.runtime_config(), yaml.as_bytes())?;
    tracing::info!("已写入占位配置（尚未添加订阅）");
    Ok(())
}

/// 读取或生成 secret 并落盘
fn load_or_create_secret(s: &Arc<AppState>) -> AppResult<String> {
    let path = s.paths.secret_file();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let t = existing.trim();
        if !t.is_empty() {
            return Ok(t.to_string());
        }
    }
    let secret = crate::kernel::ports::generate_secret();
    crate::system::paths::atomic_write(&path, secret.as_bytes())?;
    Ok(secret)
}

/// 应用完全初始化（在 Tauri setup 中调用）
pub async fn bootstrap(s: Arc<AppState>) -> AppResult<()> {
    // 1. 崩溃恢复：上次若没还原系统代理，先还原
    match s.sysproxy.recover_if_needed() {
        Ok(true) => tracing::warn!("已从上次异常退出恢复系统代理设置"),
        Ok(false) => {}
        Err(e) => tracing::warn!(err = %e, "系统代理恢复检查失败"),
    }

    // 2. 首次启动合规提示（不阻塞启动，前端弹窗）
    let settings = s.settings.get();
    if settings.compliance_accepted_at.is_none() {
        tracing::info!("首次启动：需要用户确认合规提示");
    }

    // 3. 按设置启动内核
    if settings.start_core_on_launch {
        match start_core(&s).await {
            Ok(()) => tracing::info!("内核已随应用启动"),
            Err(e) => {
                tracing::error!(err = %e, "启动内核失败");
                s.emit(
                    "core:state",
                    serde_json::json!({
                        "state": "failed",
                        "error": e.message,
                        "code": e.code.as_str(),
                    }),
                );
            }
        }
    }

    // 4. 启动调度器
    start_scheduler(&s);

    // 5. 内核看门狗
    s.kernel.spawn_watchdog();

    Ok(())
}

/// 装配并启动调度器
pub fn start_scheduler(s: &Arc<AppState>) {
    // 已存在则不重复创建
    if s.scheduler.read().is_some() {
        return;
    }
    let Some(speedtest) = s.speedtest.read().clone() else {
        tracing::warn!("测速引擎未就绪，调度器延后启动");
        return;
    };

    let (tx, _) = tokio::sync::broadcast::channel(64);
    let sched = crate::scheduler::Scheduler::new(
        Arc::clone(&s.subs),
        Arc::clone(&s.settings),
        Arc::clone(&s.kernel),
        speedtest,
        Arc::clone(&s.traffic),
        tx,
    );
    // 流量采集要用内核 API：内核已 attach 才给，否则保持 None
    sched.set_api(s.api.read().clone());
    sched.spawn();
    *s.scheduler.write() = Some(sched);
    tracing::info!("调度器已装配");
}

/// 优雅退出：还原系统代理 + 停内核
pub async fn shutdown(s: &Arc<AppState>) {
    tracing::info!("开始退出清理");
    if let Some(sched) = s.scheduler.read().clone() {
        sched.request_stop();
    }
    if s.sysproxy.read_state().managed_by_us {
        match s.sysproxy.disable() {
            Ok(_) => tracing::info!("系统代理已还原"),
            Err(e) => tracing::error!(err = %e, "系统代理还原失败（下次启动会自动恢复）"),
        }
    }
    if let Err(e) = s.kernel.stop().await {
        tracing::warn!(err = %e, "停止内核失败");
    }
    tracing::info!("退出清理完成");
}

/// 构建托盘菜单
pub fn build_tray(app: &tauri::AppHandle) -> tauri::Result<tauri::tray::TrayIcon> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let toggle_proxy = MenuItem::with_id(app, "toggle_proxy", "切换系统代理", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart_core", "重启内核", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show, &toggle_proxy, &restart, &sep, &quit])?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("缺少窗口图标".into())
        })?)
        .tooltip("Clash Zhengl")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                handle_tray_menu(&app, &id).await;
            });
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击显示窗口
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(tray)
}

async fn handle_tray_menu(app: &tauri::AppHandle, id: &str) {
    let state = app.state::<crate::commands::AppHandleState>();
    let s = &state.0;

    match id {
        "show" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }
        "toggle_proxy" => {
            let cur = s.sysproxy.read_state();
            let result = if cur.managed_by_us {
                s.sysproxy.disable().map(|_| "已关闭系统代理")
            } else {
                match s.kernel.actual_ports() {
                    Some((p, _)) => s.sysproxy.enable(p).map(|_| "已开启系统代理"),
                    None => Err(AppError::msg(ErrorCode::CoreNotRunning, "内核未运行")),
                }
            };
            if let Err(e) = result {
                tracing::warn!(err = %e, "托盘切换系统代理失败");
            }
            // 事件体必须带上真实状态，否则 GUI 收到空对象无法更新开关
            let st = s.sysproxy.read_state();
            let _ = app.emit(
                "sysproxy:changed",
                serde_json::json!({
                    "enabled": st.enabled,
                    "managedByUs": st.managed_by_us,
                    "server": st.server,
                }),
            );
        }
        "restart_core" => {
            let _ = s.sysproxy.disable();
            let _ = s.kernel.stop().await;
            s.detach_core();
            if let Err(e) = start_core(s).await {
                tracing::error!(err = %e, "托盘重启内核失败");
            }
            let _ = app.emit(
                "core:state",
                serde_json::json!({"state": if s.kernel.state().is_running() {"running"} else {"failed"}}),
            );
        }
        "quit" => {
            shutdown(s).await;
            app.exit(0);
        }
        _ => {}
    }
}

/// 把 AppState 的事件桥接到 Tauri 前端事件
pub fn bridge_events(app: tauri::AppHandle, mut rx: tokio::sync::broadcast::Receiver<AppEvent>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    // 用契约事件名发到前端
                    if let Err(e) = app.emit(&ev.kind, ev.payload) {
                        tracing::debug!(err = %e, kind = %ev.kind, "前端事件发送失败（窗口可能已关闭）");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "事件订阅落后，已丢弃部分事件");
                }
                Err(_) => break,
            }
        }
    });
}
