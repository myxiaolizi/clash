//! 日志初始化（契约 C.9）。
//!
//! - 滚动：`logs/app-YYYYMMDD.log`，单文件 2 MB × 保留 3 天
//! - 级别由 `AppSettings.log_level` 控制，热切换通过 `reload` handle
//! - **所有写日志的路径都必须先过 `redact::sanitize`**（在宏封装层统一处理）

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// 持有 guard 直到进程退出，否则日志会丢尾部
pub struct LogGuard {
    _file_guard: WorkerGuard,
    _stdout_guard: Option<WorkerGuard>,
}

/// 初始化日志。`level` 来自设置；`also_stdout` 在 debug 构建下为 true。
pub fn init(log_dir: &PathBuf, level: tracing::level_filters::LevelFilter, also_stdout: bool) -> Option<LogGuard> {
    std::fs::create_dir_all(log_dir).ok()?;

    // 按天滚动，文件名 app-YYYYMMDD.log
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("app")
        .filename_suffix("log")
        .max_log_files(3)
        .build(log_dir)
        .ok()?;

    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    // 过滤：默认 info，但 pp 模块压到 warn 避免刷屏
    let filter = EnvFilter::builder()
        .with_default_directive(level.into())
        .from_env_lossy()
        .add_directive("reqwest=warn".parse().ok()?)
        .add_directive("rustls=warn".parse().ok()?)
        .add_directive("hyper=warn".parse().ok()?)
        .add_directive("h2=warn".parse().ok()?);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer.with_max_level(tracing::Level::TRACE))
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .compact();

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    if also_stdout {
        let (stdout_writer, stdout_guard) = tracing_appender::non_blocking(std::io::stdout());
        let stdout_layer = tracing_subscriber::fmt::layer()
            .with_writer(stdout_writer.with_max_level(tracing::Level::TRACE))
            .with_ansi(true)
            .with_target(true)
            .compact();
        registry.with(stdout_layer).init();
        Some(LogGuard {
            _file_guard: file_guard,
            _stdout_guard: Some(stdout_guard),
        })
    } else {
        registry.init();
        Some(LogGuard {
            _file_guard: file_guard,
            _stdout_guard: None,
        })
    }
}

/// 清理超过保留期的日志（滚动 appender 已处理，此处兜底删孤儿文件）
pub fn cleanup_old_logs(log_dir: &PathBuf, keep_days: u64) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.starts_with("app-") || name.starts_with("core-")) {
            continue;
        }
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = now.duration_since(modified) {
                if age.as_secs() > keep_days * 86_400 {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}
