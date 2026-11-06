//! mihomo 子进程生命周期管理（P1 核心）。
//!
//! 关键设计：
//! 1. **Job Object 绑定子进程**：托盘退出/崩溃时内核必须一起死，
//!    否则下次启动会端口冲突（用户侧表现是"软件打不开"）。
//! 2. 健康检查：轮询 `GET /version`，区分「还在绑定端口」与「真的起不来」。
//! 3. 崩溃重启：指数退避，连续失败 N 次后停止并报错，避免无限重启风暴。
//! 4. 启动后用 `GET /configs` 回读实际端口，不一致即视为冲突。

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::sync::broadcast;

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::CoreState;
use crate::kernel::api::SelectorApi;

/// 启动超时：内核通常 300-600ms 起来，给 8 秒余量
const STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// 崩溃重启退避基数
const RESTART_BACKOFF_BASE_MS: u64 = 1000;
const RESTART_BACKOFF_MAX_MS: u64 = 30_000;
/// 连续失败多少次后停止重试
const MAX_RESTART_ATTEMPTS: u32 = 5;

/// 内核启动参数
#[derive(Debug, Clone)]
pub struct CoreLaunchSpec {
    pub exe: PathBuf,
    /// 工作目录（`-d`），内核把 cache.db/geodata 放这里
    pub workdir: PathBuf,
    /// 配置文件（`-f`）
    pub config: PathBuf,
    pub api_port: u16,
    pub mixed_port: u16,
    pub secret: String,
}

impl CoreLaunchSpec {
    pub fn from_paths(
        paths: &crate::system::paths::Paths,
        version: &str,
        api_port: u16,
        mixed_port: u16,
        secret: String,
    ) -> Self {
        Self {
            exe: paths.core_binary(version),
            // `-d` 必须是 **config.yaml 的父目录**。
            //
            // mihomo 的热重载（PUT /configs {"path": ...}）会校验目标路径属于
            // home dir（即 `-d`）或 SAFE_PATHS，否则返回：
            //   400 path is not subpath of home directory or SAFE_PATHS
            // 早期实现用 core-cache 当 workdir、config 放在 runtime/，
            // 两者是兄弟目录，热重载必然被拒 —— 表现为"应用订阅后节点不生效"。
            workdir: paths.runtime_dir(),
            config: paths.runtime_config(),
            api_port,
            mixed_port,
            secret,
        }
    }
}

/// 内核管理器
pub struct KernelSupervisor {
    spec: Mutex<Option<CoreLaunchSpec>>,
    child: Mutex<Option<Child>>,
    state: Mutex<CoreState>,
    /// 是否由用户主动停止（避免把主动停止当成崩溃去重启）
    stopping: AtomicBool,
    /// 实际生效的端口（启动后回读）
    actual_ports: Mutex<Option<(u16, u16)>>,
    events: broadcast::Sender<CoreState>,
    /// 内核 stdout/stderr 的最近若干行（启动失败时用作错误原因）
    core_output: Arc<Mutex<std::collections::VecDeque<String>>>,
}

/// 环形缓冲保留行数。只留最近 64 行足够定位启动失败原因，且内存开销可忽略。
const CORE_OUTPUT_KEEP: usize = 64;

fn push_ring(buf: &Mutex<std::collections::VecDeque<String>>, line: &str) {
    let mut b = buf.lock();
    if b.len() >= CORE_OUTPUT_KEEP {
        b.pop_front();
    }
    b.push_back(line.to_string());
}

impl KernelSupervisor {
    pub fn new(events: broadcast::Sender<CoreState>) -> Arc<Self> {
        Arc::new(Self {
            spec: Mutex::new(None),
            child: Mutex::new(None),
            state: Mutex::new(CoreState::Stopped),
            stopping: AtomicBool::new(false),
            actual_ports: Mutex::new(None),
            events,
            core_output: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        })
    }

    pub fn state(&self) -> CoreState {
        self.state.lock().clone()
    }

    pub fn actual_ports(&self) -> Option<(u16, u16)> {
        *self.actual_ports.lock()
    }

    fn set_state(&self, s: CoreState) {
        *self.state.lock() = s.clone();
        let _ = self.events.send(s);
    }

    /// 启动内核并等待健康检查通过
    pub async fn start(&self, spec: CoreLaunchSpec) -> AppResult<(u16, u16)> {
        if self.state().is_running() {
            if let Some(p) = self.actual_ports() {
                return Ok(p);
            }
        }

        if !spec.exe.exists() {
            let e = AppError::msg(
                ErrorCode::CoreNotFound,
                format!("找不到内核文件：{}", spec.exe.display()),
            );
            self.set_state(CoreState::Failed(e.message.clone()));
            return Err(e);
        }
        if !spec.config.exists() {
            let e = AppError::msg(
                ErrorCode::ConfigInvalid,
                format!("配置文件不存在：{}", spec.config.display()),
            );
            self.set_state(CoreState::Failed(e.message.clone()));
            return Err(e);
        }

        self.stopping.store(false, Ordering::SeqCst);
        self.set_state(CoreState::Starting);
        *self.spec.lock() = Some(spec.clone());

        std::fs::create_dir_all(&spec.workdir)?;

        // 启动子进程
        let mut child = self.spawn(&spec)?;
        tracing::info!(
            pid = child.id().unwrap_or(0),
            mixed = spec.mixed_port,
            api = spec.api_port,
            "内核进程已启动，等待健康检查"
        );

        // 转发内核 stdout/stderr 到日志（带 core 前缀）
        self.pipe_logs(&mut child);

        *self.child.lock() = Some(child);

        // 等待健康检查
        match self.wait_ready(&spec).await {
            Ok(()) => {
                *self.actual_ports.lock() = Some((spec.mixed_port, spec.api_port));
                self.set_state(CoreState::Running);
                tracing::info!("内核就绪");
                Ok((spec.mixed_port, spec.api_port))
            }
            Err(e) => {
                tracing::error!(err = %e, "内核启动失败，正在清理进程");
                self.kill_child().await;
                let msg = e.message.clone();
                self.set_state(CoreState::Failed(msg));
                Err(e)
            }
        }
    }

    fn spawn(&self, spec: &CoreLaunchSpec) -> AppResult<Child> {
        let mut cmd = Command::new(&spec.exe);
        cmd.arg("-d")
            .arg(&spec.workdir)
            .arg("-f")
            .arg(&spec.config)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 关键：子进程必须随父进程组一起结束
            .kill_on_drop(true);

        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW：不要弹出黑色控制台窗口
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.spawn().map_err(|e| {
            AppError::with_detail(
                ErrorCode::CoreStartFailed,
                format!("spawn 失败: {e}"),
            )
        })
    }

    /// 把内核输出转发到 tracing（避免管道写满导致内核阻塞）
    fn pipe_logs(&self, child: &mut Child) {
        if let Some(stdout) = child.stdout.take() {
            let buf = Arc::clone(&self.core_output);
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let s = crate::util::redact::sanitize(&line);
                    push_ring(&buf, &s);
                    tracing::info!(target: "core", "{}", s);
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let buf = Arc::clone(&self.core_output);
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let s = crate::util::redact::sanitize(&line);
                    push_ring(&buf, &s);
                    tracing::warn!(target: "core", "{}", s);
                }
            });
        }
    }

    /// 取内核最近输出（用于启动失败时给出真实原因）
    fn recent_core_output(&self) -> Option<String> {
        let buf = self.core_output.lock();
        if buf.is_empty() {
            return None;
        }
        // 优先找 fatal/error 行——这才是失败根因
        let key = buf
            .iter()
            .rev()
            .find(|l| {
                let low = l.to_ascii_lowercase();
                low.contains("fatal") || low.contains("error") || low.contains("parse config")
            })
            .or_else(|| buf.back());
        key.cloned()
    }

    /// 轮询 `/version` 直到就绪。
    ///
    /// 区分三种情况：
    /// - 连接被拒：内核还在绑端口，继续等
    /// - 401：secret 不一致（配置重建时没同步），立刻失败
    /// - 子进程已退出：读错误输出后失败
    async fn wait_ready(&self, spec: &CoreLaunchSpec) -> AppResult<()> {
        let api = crate::kernel::api::HttpKernelApi::new(spec.api_port, spec.secret.clone())?;
        let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
        let mut last_err = String::new();

        while tokio::time::Instant::now() < deadline {
            // 先看进程是否已经退出（比如配置错误会立刻退出）
            {
                let mut guard = self.child.lock();
                if let Some(child) = guard.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // 用内核自己吐的 fatal/error 行作为原因。
                            // 只报"请求超时"会让用户完全无从下手（实际是配置被拒）。
                            let reason = self
                                .recent_core_output()
                                .unwrap_or_else(|| "请查看 logs 目录下的内核输出".to_string());
                            return Err(AppError::msg(
                                ErrorCode::CoreStartFailed,
                                format!(
                                    "内核进程已退出（code={:?}）：{}",
                                    status.code(),
                                    crate::domain::errors::truncate(&reason, 300)
                                ),
                            ));
                        }
                        Ok(None) => {}
                        Err(e) => {
                            return Err(AppError::with_detail(ErrorCode::CoreStartFailed, e))
                        }
                    }
                }
            }

            match api.version().await {
                Ok(v) => {
                    tracing::info!(version = %v, "内核 /version 应答正常");
                    return Ok(());
                }
                Err(crate::domain::model::ApiError::Unauthorized) => {
                    return Err(AppError::msg(
                        ErrorCode::CoreApiUnauth,
                        "内核返回 401：secret 与配置不一致",
                    ));
                }
                Err(e) => {
                    last_err = format!("{e}");
                    tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
                }
            }
        }

        Err(AppError::msg(
            ErrorCode::CoreStartFailed,
            format!("启动超时（{}s），最后错误：{last_err}", STARTUP_TIMEOUT.as_secs()),
        ))
    }

    /// 校验实际监听端口与期望是否一致（防止端口冲突被内核静默改掉）
    pub async fn verify_ports(&self) -> AppResult<()> {
        let (expected_mixed, _) = self
            .actual_ports()
            .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))?;
        let spec = self
            .spec
            .lock()
            .clone()
            .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))?;

        let cfg = read_configs(spec.api_port, &spec.secret).await?;
        if let Some(actual) = cfg.get("mixed-port").and_then(|v| v.as_u64()) {
            if actual as u16 != expected_mixed {
                return Err(AppError::msg(
                    ErrorCode::CorePortExhausted,
                    format!("内核实际使用端口 {actual}，期望 {expected_mixed}（端口被占用）"),
                ));
            }
        }
        Ok(())
    }

    /// 热重载配置（`PUT /configs?force=true`）。
    ///
    /// 用 force=true 是因为我们的配置是**整体重写**的，不是增量补丁；
    /// 不带 force 时 mihomo 会拒绝部分字段变更。
    pub async fn reload_config(&self) -> AppResult<()> {
        let spec = self
            .spec
            .lock()
            .clone()
            .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))?;

        let client = crate::util::net::build_kernel_client()?;
        let url = format!(
            "http://127.0.0.1:{}/configs?force=true",
            spec.api_port
        );
        let body = serde_json::json!({ "path": spec.config.to_string_lossy() });

        let resp = client
            .put(&url)
            .bearer_auth(&spec.secret)
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AppError::with_detail(ErrorCode::CoreNotRunning, e.without_url()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::new(ErrorCode::CoreApiUnauth));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::msg(
                ErrorCode::ConfigInvalid,
                format!(
                    "热重载失败 HTTP {status}：{}",
                    crate::domain::errors::truncate(
                        &crate::util::redact::sanitize(&body),
                        200
                    )
                ),
            ));
        }
        tracing::info!("内核配置热重载成功");
        Ok(())
    }

    /// 主动停止
    pub async fn stop(&self) -> AppResult<()> {
        self.stopping.store(true, Ordering::SeqCst);
        self.set_state(CoreState::Stopping);
        self.kill_child().await;
        *self.actual_ports.lock() = None;
        self.set_state(CoreState::Stopped);
        tracing::info!("内核已停止");
        Ok(())
    }

    async fn kill_child(&self) {
        let taken = self.child.lock().take();
        if let Some(mut child) = taken {
            // 先温和尝试
            let _ = child.start_kill();
            match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!("内核未在 3 秒内退出，强制结束");
                    let _ = child.kill().await;
                }
            }
        }
    }

    /// 后台监控：进程意外退出时按退避重启
    pub fn spawn_watchdog(self: &Arc<Self>) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut attempt: u32 = 0;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;

                if this.stopping.load(Ordering::SeqCst) {
                    attempt = 0;
                    continue;
                }
                if !matches!(this.state(), CoreState::Running) {
                    continue;
                }

                // 检查进程是否还活着
                let exited = {
                    let mut guard = this.child.lock();
                    match guard.as_mut() {
                        Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                        None => false,
                    }
                };
                if !exited {
                    attempt = 0;
                    continue;
                }

                attempt += 1;
                if attempt > MAX_RESTART_ATTEMPTS {
                    this.set_state(CoreState::Failed(format!(
                        "内核连续 {MAX_RESTART_ATTEMPTS} 次异常退出，已停止自动重启"
                    )));
                    tracing::error!("达到最大重启次数，停止尝试");
                    continue;
                }

                let backoff = (RESTART_BACKOFF_BASE_MS * (1u64 << (attempt - 1)))
                    .min(RESTART_BACKOFF_MAX_MS);
                tracing::warn!(
                    attempt,
                    backoff_ms = backoff,
                    "检测到内核异常退出，准备重启"
                );
                tokio::time::sleep(Duration::from_millis(backoff)).await;

                let Some(spec) = this.spec.lock().clone() else {
                    break;
                };
                tracing::info!("正在重启内核");
                // 避免 start() 内部的 stopping 检查干扰
                this.stopping.store(false, Ordering::SeqCst);
                match this.start(spec).await {
                    Ok(_) => {
                        tracing::info!("内核重启成功");
                    }
                    Err(e) => {
                        tracing::error!(err = %e, "内核重启失败");
                    }
                }
            }
        });
    }
}

/// 读取内核 `/configs`（启动后核对端口用）。
///
/// 不扩 `SelectorApi` trait：该调用一生只在启动校验时用一次，
/// 为它污染冻结面不划算（C.6.1 冻结面改动需全链路通知）。
async fn read_configs(api_port: u16, secret: &str) -> AppResult<serde_json::Value> {
    let client = crate::util::net::build_kernel_client()?;
    let url = format!("http://127.0.0.1:{api_port}/configs");
    let resp = client
        .get(&url)
        .bearer_auth(secret)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map_err(|e| {
            AppError::with_detail(ErrorCode::CoreNotRunning, e.without_url())
        })?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::new(ErrorCode::CoreApiUnauth));
    }
    if !resp.status().is_success() {
        return Err(AppError::msg(
            ErrorCode::CoreStartFailed,
            format!("GET /configs 返回 {}", resp.status()),
        ));
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::CoreStartFailed, e.without_url()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let b1 = (RESTART_BACKOFF_BASE_MS * (1u64 << 0)).min(RESTART_BACKOFF_MAX_MS);
        let b2 = (RESTART_BACKOFF_BASE_MS * (1u64 << 1)).min(RESTART_BACKOFF_MAX_MS);
        let b5 = (RESTART_BACKOFF_BASE_MS * (1u64 << 4)).min(RESTART_BACKOFF_MAX_MS);
        assert_eq!(b1, 1000);
        assert_eq!(b2, 2000);
        assert_eq!(b5, 16_000);
        let b10 = (RESTART_BACKOFF_BASE_MS * (1u64 << 9)).min(RESTART_BACKOFF_MAX_MS);
        assert_eq!(b10, RESTART_BACKOFF_MAX_MS, "必须封顶");
    }

    #[test]
    fn supervisor_initial_state_is_stopped() {
        let (tx, _) = broadcast::channel(8);
        let s = KernelSupervisor::new(tx);
        assert_eq!(s.state(), CoreState::Stopped);
        assert!(s.actual_ports().is_none());
    }

    #[test]
    fn launch_spec_config_is_inside_workdir() {
        // 回归测试：mihomo 的热重载要求配置文件位于 `-d` 指定的 home dir 内，
        // 否则 PUT /configs 返回 400 "path is not subpath of home directory"，
        // 表现为"应用订阅后节点列表不刷新"。
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::system::paths::Paths::with_roots(
            dir.path().join("roaming"),
            dir.path().join("local"),
        );
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        let spec = CoreLaunchSpec::from_paths(&paths, "v1.19.33", 49152, 10801, "s".into());

        assert!(
            spec.config.starts_with(&spec.workdir),
            "config ({}) 必须是 workdir ({}) 的子路径",
            spec.config.display(),
            spec.workdir.display()
        );
        assert_eq!(spec.config.file_name().unwrap(), "config.yaml");
    }

    #[tokio::test]
    async fn start_fails_when_exe_missing() {
        let (tx, _) = broadcast::channel(8);
        let s = KernelSupervisor::new(tx);
        let spec = CoreLaunchSpec {
            exe: PathBuf::from("C:/nonexistent/mihomo.exe"),
            workdir: PathBuf::from("C:/nonexistent"),
            config: PathBuf::from("C:/nonexistent/config.yaml"),
            api_port: 49152,
            mixed_port: 10801,
            secret: "x".into(),
        };
        let r = s.start(spec).await;
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().code, ErrorCode::CoreNotFound);
        assert!(s.state().is_failed());
    }

    #[tokio::test]
    async fn start_fails_when_config_missing() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mihomo.exe");
        std::fs::write(&exe, b"fake").unwrap();

        let (tx, _) = broadcast::channel(8);
        let s = KernelSupervisor::new(tx);
        let spec = CoreLaunchSpec {
            exe,
            workdir: dir.path().to_path_buf(),
            config: dir.path().join("missing.yaml"),
            api_port: 49152,
            mixed_port: 10801,
            secret: "x".into(),
        };
        let r = s.start(spec).await;
        assert_eq!(r.unwrap_err().code, ErrorCode::ConfigInvalid);
    }

    #[tokio::test]
    async fn stop_is_idempotent_when_not_running() {
        let (tx, _) = broadcast::channel(8);
        let s = KernelSupervisor::new(tx);
        assert!(s.stop().await.is_ok());
        assert_eq!(s.state(), CoreState::Stopped);
    }
}
