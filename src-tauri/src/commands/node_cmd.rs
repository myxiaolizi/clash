//! 节点/测速/切换 IPC 命令（C.6）。

use std::sync::Arc;

use tauri::State;

use crate::commands::dto::*;
use crate::commands::state::AppHandleState;
use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::SwitchReason;

type R<T> = Result<T, IpcError>;

/// 列出可切换组及其节点（含最近测速结果）
pub async fn node_groups_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<Vec<GroupDto>> {
    
    let api = s.require_api()?;
    let groups = api.proxies_selectable().await.map_err(|e| {
        AppError::from(e)
    })?;

    let last = s.speedtest.read().as_ref().and_then(|e| e.last_run());
    let delays: std::collections::HashMap<String, Option<u32>> = last
        .as_ref()
        .map(|r| {
            r.results
                .iter()
                .map(|p| (p.node_key.clone(), p.delay_ms))
                .collect()
        })
        .unwrap_or_default();

    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        let mut nodes: Vec<NodeDto> = g
            .members
            .iter()
            .map(|m| NodeDto {
                name: m.clone(),
                delay_ms: delays.get(m).copied().flatten(),
                ok: delays.get(m).copied().flatten().is_some(),
                rank: None,
                error: None,
                selected: g.now.as_deref() == Some(m.as_str()),
            })
            .collect();

        // 按延迟排序，未测速的排后
        nodes.sort_by(|a, b| {
            a.delay_ms
                .unwrap_or(u32::MAX)
                .cmp(&b.delay_ms.unwrap_or(u32::MAX))
                .then_with(|| a.name.cmp(&b.name))
        });
        let mut rank = 0;
        for n in nodes.iter_mut() {
            if n.delay_ms.is_some() {
                rank += 1;
                n.rank = Some(rank);
            }
        }

        out.push(GroupDto {
            name: g.name,
            now: g.now,
            node_count: nodes.len() as u32,
            nodes,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn node_groups(state: State<'_, AppHandleState>) -> R<Vec<GroupDto>> {
    node_groups_inner(&state.0).await.map_err(IpcError::from)
}


/// 手动选择节点
pub async fn node_select_inner(s: &Arc<crate::commands::state::AppState>, group: String, node: String) -> AppResult<OpResultDto> {
    
    let api = s.require_api()?;

    // 先确认组与节点都存在，避免把 404 暴露成"成功"
    let info = api.proxy_group(&group).await.map_err(|e| match e {
        crate::domain::model::ApiError::NotFound(_) => {
            AppError::new(ErrorCode::SwitchGroupMissing)
        }
        other => AppError::from(other),
    })?;

    if !info.all.iter().any(|n| n == &node) {
        return Err(AppError::new(ErrorCode::SwitchNodeMissing));
    }

    api.set_group_selected(&group, &node)
        .await
        .map_err(AppError::from)?;

    // 记录手动覆盖，暂停该组自动切换
    let policy = s.settings.get().switch_policy;
    if let Some(engine) = s.speedtest.read().clone() {
        engine.note_manual_switch(&group, policy.manual_override_min);
    }

    // 契约 C.6：手动切换同样要进 switch_history 并发 `switch:applied`，
    // 只是 `to_delay = None`（手动切换没有测速结果，不得用 0 冒充）。
    // 早期实现只发 `node:switched`，导致"手动切过哪些节点"完全无迹可查。
    let record = crate::domain::model::SwitchRecord {
        at: crate::util::time::now_secs(),
        group: group.clone(),
        from_node: info.now.clone(),
        to_node: node.clone(),
        from_delay: None,
        to_delay: None,
        reason: crate::domain::model::SwitchReason::Manual,
    };
    if let Err(e) = s.history.push(record.clone()) {
        tracing::warn!(err = %e, "写入手动切换历史失败");
    }
    s.emit("switch:applied", serde_json::json!([record]));
    s.emit(
        "node:switched",
        serde_json::json!({"group": group, "node": node, "manual": true}),
    );
    Ok(OpResultDto::ok(format!("已切换到 {node}")))
}

#[tauri::command]
pub async fn node_select(state: State<'_, AppHandleState>, group: String, node: String) -> R<OpResultDto> {
    node_select_inner(&state.0, group, node).await.map_err(IpcError::from)
}


/// 触发一次测速
pub async fn speedtest_run_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<SpeedtestStatusDto> {
    
    let engine = s.require_speedtest()?;

    if engine.is_running() {
        return Err(AppError::new(ErrorCode::SpeedtestRunning));
    }

    s.emit("speedtest:started", serde_json::json!({}));
    let run = engine
        .run(crate::domain::model::SpeedtestTrigger::Manual)
        .await
        ?;

    let status = SpeedtestStatusDto {
        running: false,
        run_id: Some(run.run_id.clone()),
        total: run.total,
        finished: run.finished,
        ok_count: run.ok_count,
        started_at: Some(run.started_at),
        finished_at: run.finished_at,
        best_delay: run.results.iter().filter_map(|r| r.delay_ms).min(),
    };

    s.emit(
        "speedtest:done",
        serde_json::to_value(&status).unwrap_or_default(),
    );
    Ok(status)
}

#[tauri::command]
pub async fn speedtest_run(state: State<'_, AppHandleState>) -> R<SpeedtestStatusDto> {
    speedtest_run_inner(&state.0).await.map_err(IpcError::from)
}


pub async fn speedtest_status_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<SpeedtestStatusDto> {
    
    let engine = match s.speedtest.read().clone() {
        Some(e) => e,
        None => {
            return Ok(SpeedtestStatusDto {
                running: false,
                run_id: None,
                total: 0,
                finished: 0,
                ok_count: 0,
                started_at: None,
                finished_at: None,
                best_delay: None,
            })
        }
    };

    let running = engine.is_running();
    let last = engine.last_run();
    Ok(match last {
        Some(r) => SpeedtestStatusDto {
            running,
            run_id: Some(r.run_id),
            total: r.total,
            finished: r.finished,
            ok_count: r.ok_count,
            started_at: Some(r.started_at),
            finished_at: r.finished_at,
            best_delay: r.results.iter().filter_map(|p| p.delay_ms).min(),
        },
        None => SpeedtestStatusDto {
            running,
            run_id: None,
            total: 0,
            finished: 0,
            ok_count: 0,
            started_at: None,
            finished_at: None,
            best_delay: None,
        },
    })
}

#[tauri::command]
pub async fn speedtest_status(state: State<'_, AppHandleState>) -> R<SpeedtestStatusDto> {
    speedtest_status_inner(&state.0).await.map_err(IpcError::from)
}


/// 依据最近测速结果立即执行一次自动切换判定
pub async fn switch_auto_apply_inner(s: &Arc<crate::commands::state::AppState>) -> AppResult<Vec<SwitchRecordDto>> {
    let s = Arc::clone(s);

    // 关键：先取出 Arc 并丢弃所有 parking_lot 守卫，再进入 await。
    // `parking_lot::RwLockReadGuard` 不是 Send，跨 await 会让整个 future 不 Send，
    // Tauri 命令必须 Send（在多线程 runtime 上跑）。
    let engine = {
        let guard = s.speedtest.read();
        guard.clone()
    }
    .ok_or_else(|| AppError::new(ErrorCode::CoreNotRunning))?;

    let settings = s.settings.get();

    let records = engine
        .auto_switch(
            &settings.switch_policy,
            &settings.speedtest_probe_url,
            settings.speedtest_timeout_ms,
        )
        .await
        ?;

    let dtos: Vec<SwitchRecordDto> = records.iter().map(SwitchRecordDto::from).collect();
    if !dtos.is_empty() {
        s.emit(
            "switch:applied",
            serde_json::to_value(&dtos).unwrap_or_default(),
        );
    }
    Ok(dtos)
}

#[tauri::command]
pub async fn switch_auto_apply(state: State<'_, AppHandleState>) -> R<Vec<SwitchRecordDto>> {
    switch_auto_apply_inner(&state.0).await.map_err(IpcError::from)
}


/// 手动切换历史
pub async fn switch_history_inner(s: &Arc<crate::commands::state::AppState>, limit: Option<usize>) -> AppResult<Vec<SwitchRecordDto>> {
    
    Ok(s.history
        .recent(limit.unwrap_or(50).min(200))
        .iter()
        .map(SwitchRecordDto::from)
        .collect())
}

#[tauri::command]
pub async fn switch_history(state: State<'_, AppHandleState>, limit: Option<usize>) -> R<Vec<SwitchRecordDto>> {
    switch_history_inner(&state.0, limit).await.map_err(IpcError::from)
}


/// 直接指定节点并记录为手动切换（供托盘快捷切换）
pub async fn switch_manual_inner(s: &Arc<crate::commands::state::AppState>, group: String, node: String) -> AppResult<SwitchRecordDto> {
    
    let api = s.require_api()?;

    let info = api.proxy_group(&group).await.map_err(|e| match e {
        crate::domain::model::ApiError::NotFound(_) => {
            AppError::new(ErrorCode::SwitchGroupMissing)
        }
        other => AppError::from(other),
    })?;

    let from = info.now.clone();
    api.set_group_selected(&group, &node)
        .await
        .map_err(AppError::from)?;

    let policy = s.settings.get().switch_policy;
    if let Some(engine) = s.speedtest.read().clone() {
        engine.note_manual_switch(&group, policy.manual_override_min);
    }

    let rec = crate::domain::model::SwitchRecord {
        at: crate::util::time::now_secs(),
        group: group.clone(),
        from_node: from,
        to_node: node,
        from_delay: None,
        // 手动切换时无新鲜测速数据 → None，不得用 0 冒充
        to_delay: None,
        reason: SwitchReason::Manual,
    };
    let _ = s.history.push(rec.clone());

    s.emit(
        "switch:manual",
        serde_json::to_value(SwitchRecordDto::from(&rec)).unwrap_or_default(),
    );
    Ok(SwitchRecordDto::from(&rec))
}

#[tauri::command]
pub async fn switch_manual(state: State<'_, AppHandleState>, group: String, node: String) -> R<SwitchRecordDto> {
    switch_manual_inner(&state.0, group, node).await.map_err(IpcError::from)
}


/// 只读：当前组的 API 原始 kind（排障时确认是否为 Selector）
pub async fn node_group_raw_kind_inner(s: &Arc<crate::commands::state::AppState>, group: String) -> AppResult<String> {
    
    let api = s.require_api()?;
    let info = api
        .proxy_group(&group)
        .await
        .map_err(AppError::from)?;
    Ok(info.kind)
}

#[tauri::command]
pub async fn node_group_raw_kind(state: State<'_, AppHandleState>, group: String) -> R<String> {
    node_group_raw_kind_inner(&state.0, group).await.map_err(IpcError::from)
}


#[cfg(test)]
mod tests {
    use crate::domain::policy::{is_selector, KIND_SELECTOR};

    #[test]
    fn raw_kind_contract_is_camelcase_selector() {
        // 前端排障面板会展示这个值；契约要求是 API 原值
        assert_eq!(KIND_SELECTOR, "Selector");
        assert!(is_selector("Selector"));
        assert!(!is_selector("select"), "YAML 值不是 API 值");
    }
}
