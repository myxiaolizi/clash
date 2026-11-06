//! Clash Zhengl 内核守护进程（daemon）。
//!
//! # 为什么要有这个进程
//!
//! 原架构是 Tauri 单进程（Rust + WebView）。改用 WinUI 3 后，UI 是 .NET 进程，
//! 业务逻辑仍在 Rust。两者分离进程带来三个实际好处：
//!
//! 1. **UI 崩溃不拖垮内核** —— WinUI 进程异常退出时，已建立的代理连接不会断。
//! 2. **内核可独立重启** —— 排障时只需重启 daemon，不必重开窗口。
//! 3. **权限边界清晰** —— daemon 负责所有敏感操作（DPAPI、注册表、子进程）。
//!
//! # 通信方式
//!
//! 仅监听 `127.0.0.1`，端口写入 `%LOCALAPPDATA%\ClashZhengl\runtime\daemon.port`，
//! 调用方还需附带 `daemon.token` 里的随机 token（Bearer）。
//! 两个文件都只对当前用户可读 —— 本机其它用户无法冒充调用。

use std::sync::Arc;

use clash_zhengl_lib::app;
use clash_zhengl_lib::commands::state::AppState;
use clash_zhengl_lib::daemon::{self, DaemonCfg};
use clash_zhengl_lib::system::paths::Paths;

fn main() {
    // ---- 数据目录 ----
    let paths = match Paths::resolve() {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("无法解析数据目录: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = paths.ensure_all() {
        eprintln!("无法创建数据目录: {e}");
        std::process::exit(1);
    }

    // ---- 日志（与 GUI 共用同一套滚动日志，前缀区分）----
    let _log_guard = clash_zhengl_lib::util::log::init(
        &paths.logs_dir(),
        tracing::level_filters::LevelFilter::INFO,
        false,
    );
    clash_zhengl_lib::util::log::cleanup_old_logs(&paths.logs_dir(), 3);

    tracing::info!(
        version = clash_zhengl_lib::VERSION,
        pid = std::process::id(),
        "daemon 启动"
    );

    // ---- 单实例：端口文件指向的实例仍可连通就退出 ----
    if let Some(existing) = daemon::probe_existing(&paths) {
        tracing::info!(port = existing, "已有 daemon 在运行，本进程退出");
        println!("{existing}");
        return;
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(err = %e, "创建 runtime 失败");
            std::process::exit(1);
        }
    };

    let code = rt.block_on(async move {
        match run(paths).await {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!(err = %e, "daemon 异常退出");
                1
            }
        }
    });
    std::process::exit(code);
}

async fn run(paths: Arc<Paths>) -> clash_zhengl_lib::domain::errors::AppResult<()> {
    // ---- 状态构建（与旧 Tauri setup 完全相同的路径）----
    let state = AppState::new(Arc::clone(&paths))?;

    // ---- 事件订阅：必须在 bootstrap 之前，否则漏掉启动期事件 ----
    let events_rx = state.events.subscribe();

    // ---- 杀掉上一次异常退出留下的内核进程 ----
    // daemon 被强杀（任务管理器、崩溃、断电）时来不及跑 shutdown，
    // 上一次的 mihomo 会变成孤儿继续占端口。这里按可执行文件路径精确清理，
    // 只杀我们自己那份，不碰用户用别的方式装的 mihomo。
    cleanup_stale_kernel(&paths);

    // ---- bootstrap：崩溃恢复 → 加载内核 → 启动调度器 ----
    app::bootstrap(Arc::clone(&state)).await?;
    app::start_scheduler(&state);

    // ---- 启动 HTTP 服务 ----
    let (stop_tx, stop_rx) = tokio::sync::broadcast::channel::<()>(1);
    let cfg = DaemonCfg::new(Arc::clone(&paths));
    let port = daemon::serve(Arc::clone(&state), cfg, events_rx, stop_rx).await?;

    tracing::info!(port, "daemon 就绪，等待 UI 连接");
    // 端口打到 stdout，父进程（GUI）读这一行即可拿到
    println!("{port}");

    // ---- 等待停机信号 ----
    wait_stop().await;
    tracing::info!("收到停机信号，开始清理");

    let _ = stop_tx.send(());

    // 清理：关系统代理 → 停内核 → 停调度器
    app::shutdown(&state).await;

    // 清空端口文件，避免下次启动误判
    let _ = std::fs::remove_file(daemon::port_file(&paths));
    let _ = std::fs::remove_file(daemon::token_file(&paths));

    tracing::info!("daemon 已退出");
    Ok(())
}

/// 等待停机：Ctrl+C，或父进程断开时由 GUI 直接 Kill。
async fn wait_stop() {
    let _ = tokio::signal::ctrl_c().await;
}

/// 清理上一次异常退出残留的内核进程。
///
/// 只处理**我们自己目录下**的 mihomo（按可执行文件全路径匹配），
/// 不用进程名模糊匹配 —— 免得把用户从别处装的 mihomo 也杀掉。
/// 任何失败都只记日志，绝不阻断启动。
#[cfg(windows)]
fn cleanup_stale_kernel(paths: &Paths) {
    use std::os::windows::process::CommandExt;

    // 内核实际在 runtime\core\<version>\mihomo.exe
    let core_root = paths.core_root().to_string_lossy().to_lowercase();
    if !core_root.is_empty() && !std::path::Path::new(&paths.core_root()).exists() {
        return;
    }

    let output = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq mihomo.exe"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();

    let Ok(output) = output else {
        tracing::debug!("清理残留内核：tasklist 执行失败，跳过");
        return;
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut pids: Vec<u32> = Vec::new();
    for line in text.lines() {
        // CSV 形如 "mihomo.exe","12345","Console","1","28,000 K"
        if let Some(pid) = line
            .split("\",\"")
            .nth(1)
            .and_then(|s| s.trim_matches('"').parse::<u32>().ok())
        {
            pids.push(pid);
        }
    }

    if pids.is_empty() {
        return;
    }

    // 一次性查所有 PID 的路径，避免逐个起 powershell
    let script = pids
        .iter()
        .map(|p| {
            format!("Get-Process -Id {p} -ErrorAction SilentlyContinue | \
                     ForEach-Object {{ \"$($_.Id)|$($_.Path)\" }}")
        })
        .collect::<Vec<_>>()
        .join("; ");

    let q = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output();

    let Ok(q) = q else { return };
    let listed = String::from_utf8_lossy(&q.stdout);

    let mut killed = 0usize;
    for line in listed.lines() {
        let Some((pid_s, path_s)) = line.trim().split_once('|') else {
            continue;
        };
        let (Ok(pid), path_lower) = (pid_s.trim().parse::<u32>(), path_s.trim().to_lowercase())
        else {
            continue;
        };

        // 只杀位于我们 core 目录下的进程
        if !path_lower.starts_with(&core_root) {
            continue;
        }

        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        killed += 1;
    }

    if killed > 0 {
        tracing::info!(killed, "已清理上一次残留的内核进程");
    }
}

#[cfg(not(windows))]
fn cleanup_stale_kernel(_paths: &Paths) {}
