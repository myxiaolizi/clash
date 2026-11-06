//! 订阅相关 IPC 命令（C.6）。

use std::sync::Arc;

use tauri::State;

use crate::commands::dto::*;
use crate::commands::state::AppHandleState;
use crate::domain::errors::{AppError, ErrorCode};

type R<T> = Result<T, IpcError>;

fn st<'a>(s: &'a State<'_, AppHandleState>) -> &'a Arc<crate::commands::state::AppState> {
    &s.0
}

#[tauri::command]
pub async fn subscription_list(state: State<'_, AppHandleState>) -> R<Vec<SubscriptionDto>> {
    let s = st(&state);
    Ok(s.subs.list().iter().map(SubscriptionDto::from).collect())
}

#[tauri::command]
pub async fn subscription_add(
    state: State<'_, AppHandleState>,
    name: String,
    url: String,
) -> R<SubscriptionDto> {
    let s = st(&state);
    let sub = s.subs.add(&name, &url).map_err(IpcError::from)?;
    s.emit(
        "subscription:added",
        serde_json::json!({"uid": sub.uid, "name": sub.name}),
    );
    Ok(SubscriptionDto::from(&sub))
}

#[tauri::command]
pub async fn subscription_remove(state: State<'_, AppHandleState>, uid: String) -> R<OpResultDto> {
    let s = st(&state);
    s.subs.remove(&uid).map_err(IpcError::from)?;
    s.emit("subscription:removed", serde_json::json!({"uid": uid}));
    Ok(OpResultDto::ok("已删除"))
}

#[tauri::command]
pub async fn subscription_update(
    state: State<'_, AppHandleState>,
    uid: String,
    name: Option<String>,
    enabled: Option<bool>,
    interval_min: Option<u32>,
) -> R<SubscriptionDto> {
    let s = st(&state);
    let mut sub = if let Some(n) = name {
        s.subs.rename(&uid, &n).map_err(IpcError::from)?
    } else {
        s.subs_store
            .get(&uid)
            .ok_or_else(|| IpcError::from(AppError::new(ErrorCode::NotFound)))?
    };
    if let Some(e) = enabled {
        sub = s.subs.set_enabled(&uid, e).map_err(IpcError::from)?;
    }
    if let Some(i) = interval_min {
        sub = s.subs.update_interval(&uid, i).map_err(IpcError::from)?;
    }
    Ok(SubscriptionDto::from(&sub))
}

/// 拉取单个订阅
#[tauri::command]
pub async fn subscription_fetch(
    state: State<'_, AppHandleState>,
    uid: String,
    force: Option<bool>,
) -> R<SubscriptionDto> {
    let s = st(&state);
    let parsed = s
        .subs
        .fetch_one(&uid, force.unwrap_or(false))
        .await
        .map_err(IpcError::from)?;

    let sub = s
        .subs_store
        .get(&uid)
        .ok_or_else(|| IpcError::from(AppError::new(ErrorCode::NotFound)))?;

    s.emit(
        "subscription:fetched",
        serde_json::json!({
            "uid": uid,
            "nodeCount": parsed.node_count(),
        }),
    );
    Ok(SubscriptionDto::from(&sub))
}

/// 拉取全部（用户点"全部更新"）
#[tauri::command]
pub async fn subscription_fetch_all(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    let s = st(&state);
    let results = s.subs.fetch_due(true).await;
    let total = results.len();
    let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = total - ok;

    s.emit(
        "subscription:fetchedAll",
        serde_json::json!({"total": total, "ok": ok, "failed": failed}),
    );

    let msg = if failed == 0 {
        format!("已更新 {ok} 个订阅")
    } else {
        format!("更新完成：成功 {ok}，失败 {failed}")
    };
    Ok(OpResultDto { ok: failed == 0, message: msg })
}

#[tauri::command]
pub async fn subscription_traffic(state: State<'_, AppHandleState>) -> R<TrafficSummaryDto> {
    let s = st(&state);
    let summary = s.subs.traffic_summary();
    Ok(TrafficSummaryDto::from(&summary))
}

/// 手动应用当前配置到内核（无需等定时任务）
#[tauri::command]
pub async fn subscription_apply(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    let s = st(&state);

    let ports = s
        .kernel
        .actual_ports()
        .ok_or_else(|| IpcError::from(AppError::new(ErrorCode::CoreNotRunning)))?;
    let secret = s.secret.read().clone();
    if secret.is_empty() {
        return Err(IpcError::from(AppError::new(ErrorCode::CoreApiUnauth)));
    }

    let gen = s
        .subs
        .build_config(ports.0, ports.1, &secret)
        .map_err(IpcError::from)?
        .ok_or_else(|| IpcError::from(AppError::new(ErrorCode::NoSubscription)))?;

    s.subs.write_config(&gen).map_err(IpcError::from)?;
    s.kernel.reload_config().await.map_err(IpcError::from)?;

    s.emit(
        "subscription:applied",
        serde_json::json!({"nodes": gen.node_names.len()}),
    );
    Ok(OpResultDto::ok(format!(
        "配置已生效，共 {} 个节点",
        gen.node_names.len()
    )))
}

/// 预览生成的配置（排障用，前端"查看当前配置"）
#[tauri::command]
pub async fn subscription_preview_config(state: State<'_, AppHandleState>) -> R<String> {
    let s = st(&state);
    let ports = s.kernel.actual_ports().unwrap_or((10801, 49152));
    let secret = s.secret.read().clone();
    let gen = s
        .subs
        .build_config(ports.0, ports.1, &secret)
        .map_err(IpcError::from)?;
    match gen {
        Some(g) => Ok(g.yaml),
        None => Ok(String::new()),
    }
}

#[cfg(test)]
mod tests {
    // 命令层薄，逻辑已在 manager 测试覆盖。
    // 这里仅验证错误码传递不变形。

    use crate::commands::dto::IpcError;
    use crate::domain::errors::{AppError, ErrorCode};

    #[test]
    fn ipc_error_preserves_code() {
        let e = AppError::new(ErrorCode::NoSubscription);
        let ipc: IpcError = e.into();
        assert_eq!(ipc.0.code, "E_NO_SUBSCRIPTION");
        assert!(ipc.0.retryable == false);
    }
}
