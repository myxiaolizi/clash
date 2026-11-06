//! 可执行入口：单实例、日志、Tauri 启动、退出清理。

// 发布版不弹控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use clash_zhengl_lib::app;
use clash_zhengl_lib::commands::state::{AppHandleState, AppState};
use clash_zhengl_lib::system::paths::Paths;
use tauri::Manager;

fn main() {
    // ---- 1. 解析路径并初始化日志（越早越好，后续错误才能落盘）----
    let paths = match Paths::resolve() {
        Ok(p) => {
            let _ = p.ensure_all();
            Arc::new(p)
        }
        Err(e) => {
            eprintln!("无法解析应用数据目录：{e}");
            std::process::exit(1);
        }
    };

    // 先读一次设置拿日志级别（读失败用默认 Info）
    let log_level = std::fs::read_to_string(paths.settings_file())
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("log_level")
                .and_then(|l| l.as_str())
                .map(|s| s.to_string())
        })
        .map(|s| match s.as_str() {
            "error" => tracing::level_filters::LevelFilter::ERROR,
            "warn" => tracing::level_filters::LevelFilter::WARN,
            "debug" => tracing::level_filters::LevelFilter::DEBUG,
            _ => tracing::level_filters::LevelFilter::INFO,
        })
        .unwrap_or(tracing::level_filters::LevelFilter::INFO);

    // guard 必须在 main 全程存活，否则日志尾部会丢
    let _log_guard = clash_zhengl_lib::util::log::init(
        &paths.logs_dir(),
        log_level,
        cfg!(debug_assertions),
    );

    tracing::info!(
        version = clash_zhengl_lib::VERSION,
        arch = std::env::consts::ARCH,
        data_dir = %paths.roaming.display(),
        "Clash Zhengl 启动"
    );
    clash_zhengl_lib::util::log::cleanup_old_logs(&paths.logs_dir(), 3);

    // ---- 2. 构建 Tauri 应用 ----
    let result = tauri::Builder::default()
        // 单实例：第二个实例唤起已有窗口后退出
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(clash_zhengl_lib::commands::register_handlers())
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();

            // 3.1 构建全局状态
            let state = tauri::async_runtime::block_on(async {
                AppState::new(Arc::clone(&paths))
            })
            .map_err(|e| {
                tracing::error!(err = %e, "初始化应用状态失败");
                e.to_string()
            })?;

            // 3.2 事件桥接（先订阅再启动，避免漏掉启动期事件）
            let rx = state.events.subscribe();
            app::bridge_events(handle.clone(), rx);

            tauri_app.manage(AppHandleState(Arc::clone(&state)));

            // 3.3 托盘
            match app::build_tray(&handle) {
                Ok(_tray) => {
                    // 托盘由 Tauri 持有生命周期，无需手动保存
                }
                Err(e) => tracing::error!(err = %e, "创建托盘失败（继续运行）"),
            }

            // 3.4 显示主窗口（首次启动若非静默模式）
            {
                let settings = state.settings.get();
                let silent = settings.silent_start && settings.auto_launch;
                if let Some(w) = handle.get_webview_window("main") {
                    if !silent {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                if silent {
                    tracing::info!("静默启动：主窗口保持隐藏，仅托盘可用");
                }
            }

            // 3.5 后台启动内核 + 调度器（不阻塞 UI 显示）
            let boot_state = Arc::clone(&state);
            let exit_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = app::bootstrap(Arc::clone(&boot_state)).await {
                    tracing::error!(err = %e, "应用初始化未完成");
                }
            });

            // 3.6 窗口关闭拦截（close_to_tray 时隐藏而非退出）
            if let Some(w) = handle.get_webview_window("main") {
                let w2 = w.clone();
                let state_for_close = Arc::clone(&state);
                let app_handle = exit_handle.clone();
                w.on_window_event(move |ev| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = ev {
                        let to_tray = state_for_close.settings.get().close_to_tray;
                        if to_tray {
                            api.prevent_close();
                            let _ = w2.hide();
                            tracing::debug!("窗口已隐藏到托盘");
                        } else {
                            // 真退出：交给退出钩子清理
                            let app = app_handle.clone();
                            tauri::async_runtime::spawn(async move {
                                app.exit(0);
                            });
                        }
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!());

    let tauri_app = match result {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(err = %e, "Tauri 构建失败");
            eprintln!("启动失败：{e}");
            std::process::exit(1);
        }
    };

    // ---- 4. 运行，并在退出时清理 ----
    tauri_app.run(|handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            // 退出前同步清理（block_on 保证清理完成才真正退出）
            if let Some(state) = handle.try_state::<AppHandleState>() {
                let s = Arc::clone(&state.0);
                tauri::async_runtime::block_on(async move {
                    app::shutdown(&s).await;
                });
            }
        }
    });
}
