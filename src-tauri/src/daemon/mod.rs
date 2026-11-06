//! 本地 IPC HTTP 服务：把原 Tauri 命令层暴露为 `127.0.0.1` 上的 JSON 接口。
//!
//! # 契约
//!
//! 业务调用统一走 `POST /rpc`，body 为 `{"cmd": "<命令名>", "args": {...}}`，
//! 响应 `{"ok": true, "data": ...}` 或 `{"ok": false, "error": {...}}`。
//!
//! **命令名与参数与 C.6 契约逐字一致**，C# 侧只需按契约表调用，
//! 不需要为 WinUI 另建一套命名 —— 已有 228 个测试的语义仍然有效。
//!
//! 事件推送走 SSE：`GET /events`。选 SSE 而非 WebSocket 是因为单向推送足够，
//! C# 用 HttpClient 就能消费，不必额外引入 WebSocket 依赖。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::commands::state::AppState;
use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::system::paths::Paths;

/// 统一响应体类型：JSON 与 SSE 混用，因此装箱成动态 body。
type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;
type Resp = Response<BoxBody>;

fn full_body(b: Bytes) -> BoxBody {
    Full::new(b).map_err(|never| match never {}).boxed()
}

// ============================================================================
//  配置
// ============================================================================

pub struct DaemonCfg {
    pub paths: Arc<Paths>,
    /// 监听端口；0 表示由系统分配（推荐，避免固定端口冲突）
    pub bind_port: u16,
}

impl DaemonCfg {
    pub fn new(paths: Arc<Paths>) -> Self {
        Self {
            paths,
            bind_port: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
struct RpcReq {
    cmd: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: String,
    message: String,
    detail: Option<String>,
    retryable: bool,
}

impl From<AppError> for ErrorBody {
    fn from(e: AppError) -> Self {
        Self {
            code: e.code.as_str().to_string(),
            message: e.message,
            detail: e.detail,
            retryable: e.code.retryable(),
        }
    }
}

// ============================================================================
//  token / 端口文件
// ============================================================================

pub const PORT_FILE: &str = "daemon.port";
pub const TOKEN_FILE: &str = "daemon.token";

pub fn port_file(paths: &Paths) -> std::path::PathBuf {
    paths.runtime_dir().join(PORT_FILE)
}

pub fn token_file(paths: &Paths) -> std::path::PathBuf {
    paths.runtime_dir().join(TOKEN_FILE)
}

/// 探测是否已有 daemon 在跑（端口文件 + 实际连通性双验证）
pub fn probe_existing(paths: &Paths) -> Option<u16> {
    let port = read_port(paths)?;
    std::net::TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(300),
    )
    .ok()
    .map(|_| port)
}

pub fn read_port(paths: &Paths) -> Option<u16> {
    let s = std::fs::read_to_string(port_file(paths)).ok()?;
    s.trim().parse().ok()
}

pub fn read_token(paths: &Paths) -> Option<String> {
    std::fs::read_to_string(token_file(paths))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_port(paths: &Paths, port: u16) -> AppResult<()> {
    crate::system::paths::atomic_write(&port_file(paths), port.to_string().as_bytes())
}

fn write_token(paths: &Paths, token: &str) -> AppResult<()> {
    crate::system::paths::atomic_write(&token_file(paths), token.as_bytes())
}

/// 每次启动都换新 token —— 进程重启后旧 token 立即失效
fn gen_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    hex::encode((0..16).map(|_| rng.gen::<u8>()).collect::<Vec<u8>>())
}

// ============================================================================
//  服务
// ============================================================================

/// 启动服务并返回监听端口。
///
/// `stop_tx` 是停机信号发送端：`POST /shutdown` 会向它发一次，
/// `daemon.rs` 的 `wait_stop` 也在等同一个信号 —— 这样无论用户点
/// "退出"（HTTP）还是 Ctrl+C，走的都是同一条优雅退出路径。
pub async fn serve(
    state: Arc<AppState>,
    cfg: DaemonCfg,
    events_rx: broadcast::Receiver<crate::commands::state::AppEvent>,
    stop_tx: broadcast::Sender<()>,
) -> AppResult<u16> {
    let token = gen_token();
    write_token(&cfg.paths, &token)?;

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", cfg.bind_port))
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::Internal, format!("绑定端口失败: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?
        .port();
    write_port(&cfg.paths, port)?;

    tracing::info!(port, "HTTP 服务已监听 127.0.0.1");

    // SSE 事件广播：把内核状态与调度器事件转成 JSON 文本
    let (sse_tx, _) = broadcast::channel::<String>(64);
    spawn_event_bridge(events_rx, sse_tx.clone());

    let accept_token = token.clone();
    let accept_state = Arc::clone(&state);
    let accept_sse = sse_tx.clone();
    let accept_stop = stop_tx.clone();

    // accept 循环：每条连接独立 spawn，单连接阻塞不影响其它调用
    let accept = async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(err = %e, "accept 失败，1 秒后重试");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            };
            let st = Arc::clone(&accept_state);
            let tk = accept_token.clone();
            let sse = accept_sse.clone();
            let sd = accept_stop.clone();
            tokio::spawn(async move {
                let svc = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let st = Arc::clone(&st);
                    let tk = tk.clone();
                    let sse = sse.clone();
                    let sd = sd.clone();
                    async move { Ok::<_, std::convert::Infallible>(route(st, tk, sse, sd, req).await) }
                });
                if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(hyper_util::rt::TokioIo::new(stream), svc)
                .await
                {
                    tracing::debug!(err = %e, "连接结束");
                }
            });
        }
    };

    let mut stop_rx = stop_tx.subscribe();
    tokio::spawn(async move {
        tokio::select! {
            _ = accept => {}
            _ = stop_rx.recv() => {
                tracing::info!("daemon 停止接受新连接");
            }
        }
    });

    Ok(port)
}

/// 把内部 `CoreState` 广播桥接成 SSE 文本
fn spawn_event_bridge(
    mut rx: broadcast::Receiver<crate::commands::state::AppEvent>,
    tx: broadcast::Sender<String>,
) {
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = json!({
                        "event": ev.kind,
                        "data": ev.payload,
                    });
                    let _ = tx.send(format!("data: {payload}\n\n"));
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(skipped = n, "SSE 订阅者落后，已跳过中间事件");
                }
            }
        }
    });
}

// ============================================================================
//  路由
// ============================================================================

async fn route(
    state: Arc<AppState>,
    token: String,
    sse_tx: broadcast::Sender<String>,
    stop_tx: broadcast::Sender<()>,
    req: Request<Incoming>,
) -> Resp {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // ---- 健康检查：不需要 token，供 UI 启动时探测 ----
    if path == "/health" {
        return json_resp(
            StatusCode::OK,
            &json!({"ok": true, "data": {"version": crate::VERSION}}),
        );
    }

    // ---- 鉴权：除 /health 外全部要求 Bearer token ----
    if !check_auth(req.headers(), &token) {
        return json_resp(
            StatusCode::UNAUTHORIZED,
            &json!({"ok": false, "error": {"code": "E_UNAUTHORIZED", "message": "鉴权失败", "detail": null, "retryable": false}}),
        );
    }

    match (method, path.as_str()) {
        // ---- SSE 事件流 ----
        (Method::GET, "/events") => {
            let rx = sse_tx.subscribe();
            sse_response(rx)
        }

        // ---- 优雅退出 ----
        //
        // GUI 退出时必须让 daemon 一起退出，否则 mihomo 会变成孤儿进程
        // 继续占着 mixed-port，下次启动必然端口冲突。
        // 约定：先回响应（让 GUI 知道收到了），再发停机信号。
        (Method::POST, "/shutdown") => {
            tracing::info!("收到 /shutdown 请求，准备退出");
            let _ = state.events.send(crate::commands::state::AppEvent {
                kind: "daemon:shutdown".into(),
                payload: json!({}),
                at: crate::util::time::now_secs(),
            });
            let _ = stop_tx.send(());
            json_resp(StatusCode::OK, &json!({"ok": true, "data": {"shuttingDown": true}}))
        }

        // ---- 业务 RPC ----
        (Method::POST, "/rpc") => {
            let body = match req.into_body().collect().await {
                Ok(c) => c.to_bytes(),
                Err(e) => {
                    return json_resp(
                        StatusCode::BAD_REQUEST,
                        &json!({"ok": false, "error": {"code": "E_INTERNAL", "message": format!("读取 body 失败: {e}"), "detail": null, "retryable": false}}),
                    )
                }
            };
            let rpc: RpcReq = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(e) => {
                    return json_resp(
                        StatusCode::BAD_REQUEST,
                        &json!({"ok": false, "error": {"code": "E_INTERNAL", "message": format!("JSON 解析失败: {e}"), "detail": null, "retryable": false}}),
                    )
                }
            };
            let resp = dispatch(state, &rpc.cmd, rpc.args).await;
            json_resp(StatusCode::OK, &resp)
        }

        _ => json_resp(
            StatusCode::NOT_FOUND,
            &json!({"ok": false, "error": {"code": "E_NOT_FOUND", "message": "接口不存在", "detail": null, "retryable": false}}),
        ),
    }
}

/// SSE 响应：用 mpsc + `StreamBody` 实现，避免引入 async-stream 依赖。
///
/// 前端断开时写端会收到错误，转发任务随即退出，不会泄漏。
fn sse_response(mut rx: broadcast::Receiver<String>) -> Resp {
    let (tx, body_rx) = tokio::sync::mpsc::channel::<Result<hyper::body::Frame<Bytes>, std::io::Error>>(32);

    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let frame = hyper::body::Frame::data(Bytes::from(msg));
                    if tx.send(Ok(frame)).await.is_err() {
                        break; // 客户端已断开
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(skipped = n, "SSE 订阅者落后");
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(body_rx);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream; charset=utf-8")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(http_body_util::StreamBody::new(stream).map_err(|e| std::io::Error::other(e.to_string())).boxed())
        .unwrap_or_else(|_| Response::new(full_body(Bytes::new())))
}

fn check_auth(headers: &hyper::HeaderMap, token: &str) -> bool {
    headers
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| constant_time_eq(t, token))
        .unwrap_or(false)
}

/// 常量时间比较，避免 token 通过响应时间被逐字节猜出
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn json_resp(status: StatusCode, v: &Value) -> Resp {
    let body = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        // 必须显式声明 charset=utf-8。
        //
        // RFC 8259 规定 JSON 就是 UTF-8，本不该需要这个参数；但 Windows 的
        // WinHTTP / HttpClient 在 Content-Type **没有 charset** 时会按
        // ISO-8859-1 解码 —— 结果所有中文都变成 ????? 或乱码。
        // 这个坑只在含非 ASCII 的响应上暴露，纯英文测试永远发现不了。
        .header("content-type", "application/json; charset=utf-8")
        .body(full_body(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(full_body(Bytes::new())))
}

// ============================================================================
//  命令分发
// ============================================================================

/// 把 C.6 契约里的命令名映射到既有实现。
///
/// 这一层刻意保持「薄」：只做参数解包与错误包装，业务全在 `AppState` 里，
/// 保证 daemon 与旧 Tauri 路径的行为完全一致。
async fn dispatch(state: Arc<AppState>, cmd: &str, args: Value) -> Value {
    match call(state, cmd, args).await {
        Ok(v) => json!({"ok": true, "data": v}),
        Err(e) => {
            let body = ErrorBody::from(e);
            serde_json::to_value(json!({"ok": false, "error": body}))
                .unwrap_or_else(|_| json!({"ok": false, "error": {"code": "E_INTERNAL"}}))
        }
    }
}

include!("dispatch.rs");
