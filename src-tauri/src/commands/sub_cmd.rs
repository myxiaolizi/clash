//! 订阅相关 IPC 命令（C.6）。

use std::sync::Arc;

use tauri::State;

use crate::commands::dto::*;
use crate::commands::state::AppHandleState;
use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::subscription::manager::SubMetaInput;

type R<T> = Result<T, IpcError>;

pub async fn subscription_list_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<Vec<SubscriptionDto>> {
    
    Ok(s.subs.list().iter().map(SubscriptionDto::from).collect())
}

#[tauri::command]
pub async fn subscription_list(state: State<'_, AppHandleState>) -> R<Vec<SubscriptionDto>> {
    subscription_list_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn subscription_add_inner(s: &Arc<crate::commands::state::AppState>, name: String, url: String) -> AppResult<SubscriptionDto> {
    
    let sub = s.subs.add(&name, &url)?;
    s.emit(
        "subscription:added",
        serde_json::json!({"uid": sub.uid, "name": sub.name}),
    );
    Ok(SubscriptionDto::from(&sub))
}

#[tauri::command]
pub async fn subscription_add(state: State<'_, AppHandleState>, name: String, url: String) -> R<SubscriptionDto> {
    subscription_add_inner(&state.0, name, url).await.map_err(IpcError::from)
}


/// 新增订阅（带管理信息）
pub async fn subscription_add_ex_inner(
    s: &Arc<crate::commands::state::AppState>,
    name: String,
    url: String,
    username: Option<String>,
    password: Option<String>,
    homepage: Option<String>,
    remark: Option<String>,
) -> AppResult<SubscriptionDto> {
    let meta = SubMetaInput {
        username,
        password,
        homepage,
        remark,
    };
    let sub = s.subs.add_with_meta(&name, &url, meta)?;
    s.emit(
        "subscription:added",
        serde_json::json!({"uid": sub.uid, "name": sub.name}),
    );
    Ok(SubscriptionDto::from(&sub))
}

#[tauri::command]
pub async fn subscription_add_ex(
    state: State<'_, AppHandleState>,
    name: String,
    url: String,
    username: Option<String>,
    password: Option<String>,
    homepage: Option<String>,
    remark: Option<String>,
) -> R<SubscriptionDto> {
    subscription_add_ex_inner(
        &state.0, name, url, username, password, homepage, remark,
    )
    .await
    .map_err(IpcError::from)
}


pub async fn subscription_remove_inner(s: &Arc<crate::commands::state::AppState>, uid: String) -> AppResult<OpResultDto> {
    
    s.subs.remove(&uid)?;
    s.emit("subscription:removed", serde_json::json!({"uid": uid}));
    Ok(OpResultDto::ok("已删除"))
}

#[tauri::command]
pub async fn subscription_remove(state: State<'_, AppHandleState>, uid: String) -> R<OpResultDto> {
    subscription_remove_inner(&state.0, uid).await.map_err(IpcError::from)
}


pub async fn subscription_update_inner(s: &Arc<crate::commands::state::AppState>, uid: String, name: Option<String>, enabled: Option<bool>, interval_min: Option<u32>) -> AppResult<SubscriptionDto> {
    
    let mut sub = if let Some(n) = name {
        s.subs.rename(&uid, &n)?
    } else {
        s.subs_store
            .get(&uid)
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?
    };
    if let Some(e) = enabled {
        sub = s.subs.set_enabled(&uid, e)?;
    }
    if let Some(i) = interval_min {
        sub = s.subs.update_interval(&uid, i)?;
    }
    Ok(SubscriptionDto::from(&sub))
}

#[tauri::command]
pub async fn subscription_update(state: State<'_, AppHandleState>, uid: String, name: Option<String>, enabled: Option<bool>, interval_min: Option<u32>) -> R<SubscriptionDto> {
    subscription_update_inner(&state.0, uid, name, enabled, interval_min).await.map_err(IpcError::from)
}


/// 更新订阅的管理信息（用户名 / 密码 / 机场地址 / 备注）。
///
/// 语义：参数缺席 = 保持原值；传空串 = 清空该字段。
/// 密码**只进不出**：本命令从不返回密码明文。
pub async fn subscription_update_meta_inner(
    s: &Arc<crate::commands::state::AppState>,
    uid: String,
    username: Option<String>,
    password: Option<String>,
    homepage: Option<String>,
    remark: Option<String>,
) -> AppResult<SubscriptionDto> {
    let meta = SubMetaInput {
        username,
        password,
        homepage,
        remark,
    };
    let sub = s.subs.update_meta(&uid, meta)?;
    Ok(SubscriptionDto::from(&sub))
}

#[tauri::command]
pub async fn subscription_update_meta(
    state: State<'_, AppHandleState>,
    uid: String,
    username: Option<String>,
    password: Option<String>,
    homepage: Option<String>,
    remark: Option<String>,
) -> R<SubscriptionDto> {
    subscription_update_meta_inner(&state.0, uid, username, password, homepage, remark)
        .await
        .map_err(IpcError::from)
}


/// 取订阅的机场登录密码明文（用户点「显示密码」时才调用）。
///
/// 单独开一个命令而不是塞进 `SubscriptionDto`：DTO 会被 list 命令
/// 一次性批量返回并常驻前端内存/日志，密码一旦进去就收不回来了。
pub async fn subscription_reveal_password_inner(
    s: &Arc<crate::commands::state::AppState>,
    uid: String,
) -> AppResult<String> {
    let sub = s
        .subs_store
        .get(&uid)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
    if sub.password_cipher.trim().is_empty() {
        return Ok(String::new());
    }
    crate::store::crypto::unprotect(&sub.password_cipher)
}

#[tauri::command]
pub async fn subscription_reveal_password(
    state: State<'_, AppHandleState>,
    uid: String,
) -> R<String> {
    subscription_reveal_password_inner(&state.0, uid)
        .await
        .map_err(IpcError::from)
}


/// 拉取单个订阅
pub async fn subscription_fetch_inner(s: &Arc<crate::commands::state::AppState>, uid: String, force: Option<bool>) -> AppResult<SubscriptionDto> {
    
    let parsed = s
        .subs
        .fetch_one(&uid, force.unwrap_or(false))
        .await
        ?;

    let sub = s
        .subs_store
        .get(&uid)
        .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;

    s.emit(
        "subscription:fetched",
        serde_json::json!({
            "uid": uid,
            "nodeCount": parsed.node_count(),
        }),
    );

    // ★ 关键修复：拉取成功必须立刻重建配置并热重载内核。
    //
    // 早期只在 `subscription_apply` 里做这一步，用户加完订阅点"更新"
    // 之后界面显示"已更新 N 个节点"，但节点列表是空的 ——
    // 因为 runtime/config.yaml 还是上一次的内容，内核压根没拿到新节点。
    // 内核未运行时静默跳过（下次 start_core 会用最新缓存重建）。
    if let Err(e) = apply_config_inner(s).await {
        // 不能让"配置应用失败"把"拉取成功"变成错误：
        // 用户至少该看到节点数已更新，配置问题在事件里提示。
        tracing::warn!(err = %e, "订阅拉取成功但配置应用失败");
        s.emit(
            "subscription:applyFailed",
            serde_json::json!({
                "uid": uid,
                "code": e.code.as_str(),
                "message": e.message,
            }),
        );
    }

    // 重新读一次：应用配置过程中订阅索引可能被更新过
    let sub = s.subs_store.get(&uid).unwrap_or(sub);
    Ok(SubscriptionDto::from(&sub))
}

#[tauri::command]
pub async fn subscription_fetch(state: State<'_, AppHandleState>, uid: String, force: Option<bool>) -> R<SubscriptionDto> {
    subscription_fetch_inner(&state.0, uid, force).await.map_err(IpcError::from)
}


/// 拉取全部（用户点"全部更新"）
pub async fn subscription_fetch_all_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<OpResultDto> {
    
    let results = s.subs.fetch_due(true).await;
    let total = results.len();
    let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = total - ok;

    s.emit(
        "subscription:fetchedAll",
        serde_json::json!({"total": total, "ok": ok, "failed": failed}),
    );

    // 同 `subscription_fetch`：有任何一个成功就要让内核看到新节点
    let mut apply_note = String::new();
    if ok > 0 {
        if let Err(e) = apply_config_inner(s).await {
            tracing::warn!(err = %e, "全部更新后配置应用失败");
            s.emit(
                "subscription:applyFailed",
                serde_json::json!({
                    "code": e.code.as_str(),
                    "message": e.message,
                }),
            );
            apply_note = format!("；配置未生效：{}", e.message);
        }
    }

    let msg = if failed == 0 {
        format!("已更新 {ok} 个订阅{apply_note}")
    } else {
        format!("更新完成：成功 {ok}，失败 {failed}{apply_note}")
    };
    Ok(OpResultDto { ok: failed == 0, message: msg })
}

#[tauri::command]
pub async fn subscription_fetch_all(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    subscription_fetch_all_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn subscription_traffic_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<TrafficSummaryDto> {
    
    let summary = s.subs.traffic_summary();
    Ok(TrafficSummaryDto::from(&summary))
}

#[tauri::command]
pub async fn subscription_traffic(state: State<'_, AppHandleState>) -> R<TrafficSummaryDto> {
    subscription_traffic_inner(&state.0).await.map_err(IpcError::from)
}


/// 重建配置并让内核热重载。
///
/// 这是「生成 → 落盘 → 热重载」的唯一实现，`subscription_apply`、
/// `subscription_fetch`、`subscription_fetch_all` 以及规则变更都走它，
/// 避免出现"某条路径忘了重载"导致配置与内核不一致。
///
/// 内核未运行时返回 `Ok(None)` 语义的静默跳过由调用方判断 ——
/// 这里直接返回 `Err(CoreNotRunning)`，让调用方决定是否忽略。
pub async fn apply_config_inner(
    s: &Arc<crate::commands::state::AppState>,
) -> AppResult<usize> {
    let ports = s
        .kernel
        .actual_ports()
        .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))?;

    // secret 有两个来源：AppState 内存（已 attach）与 runtime/secret 文件。
    // 优先用内存（权威），回退到文件（daemon 重启后可能还没 attach）。
    let mut secret = s.secret.read().clone();
    if secret.trim().is_empty() {
        secret = std::fs::read_to_string(s.paths.secret_file())
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
    }
    if secret.trim().is_empty() {
        return Err(AppError::new(ErrorCode::CoreApiUnauth));
    }

    let gen = s
        .subs
        .build_config(ports.0, ports.1, secret.trim())
        ?
        .ok_or_else(|| AppError::new(ErrorCode::NoSubscription))?;

    s.subs.write_config(&gen)?;
    s.kernel.reload_config().await?;

    s.emit(
        "subscription:applied",
        serde_json::json!({"nodes": gen.node_names.len()}),
    );
    Ok(gen.node_names.len())
}

/// 手动应用当前配置到内核（无需等定时任务）
pub async fn subscription_apply_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<OpResultDto> {
    let nodes = apply_config_inner(s).await?;
    Ok(OpResultDto::ok(format!("配置已生效，共 {nodes} 个节点")))
}

#[tauri::command]
pub async fn subscription_apply(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    subscription_apply_inner(&state.0).await.map_err(IpcError::from)
}


/// 预览生成的配置（排障用，前端"查看当前配置"）
pub async fn subscription_preview_config_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<String> {
    
    let ports = s.kernel.actual_ports().unwrap_or((10801, 49152));
    let secret = s.secret.read().clone();
    let gen = s
        .subs
        .build_config(ports.0, ports.1, &secret)
        ?;
    match gen {
        Some(g) => Ok(g.yaml),
        None => Ok(String::new()),
    }
}

#[tauri::command]
pub async fn subscription_preview_config(state: State<'_, AppHandleState>) -> R<String> {
    subscription_preview_config_inner(&state.0).await.map_err(IpcError::from)
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
