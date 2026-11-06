//! 节点/测速/切换 IPC 命令（C.6）。

use std::sync::Arc;

use tauri::State;

use crate::commands::dto::*;
use crate::commands::state::AppHandleState;
use crate::domain::errors::{AppError, ErrorCode};
use crate::domain::model::SwitchReason;

type R<T> = Result<T, IpcError>;

fn st<'a>(s: &'a State<'_, AppHandleState>) -> &'a Arc<crate::commands::state::AppState> {
    &s.0
}

/// 列出可切换组及其节点（含最近测速结果）
#[tauri::command]
pub async fn node_groups(state: State<'_, AppHandleState>) -> R<Vec<GroupDto>> {
    let s = st(&state);
    let api = s.require_api().map_err(IpcError::from)?;
    let groups = api.proxies_selectable().await.map_err(|e| {
        IpcError::from(AppError::from(e))
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

/// 手动选择节点
#[tauri::command]
pub async fn node_select(
    state: State<'_, AppHandleState>,
    group: String,
    node: String,
) -> R<OpResultDto> {
    let s = st(&state);
    let api = s.require_api().map_err(IpcError::from)?;

    // 先确认组与节点都存在，避免把 404 暴露成"成功"
    let info = api.proxy_group(&group).await.map_err(|e| match e {
        crate::domain::model::ApiError::NotFound(_) => {
            IpcError::from(AppError::new(ErrorCode::SwitchGroupMissing))
        }
        other => IpcError::from(AppError::from(other)),
    })?;

    if !info.all.iter().any(|n| n == &node) {
        return Err(IpcError::from(AppError::new(ErrorCode::SwitchNodeMissing)));
    }

    api.set_group_selected(&group, &node)
        .await
        .map_err(|e| IpcError::from(AppError::from(e)))?;

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

/// 触发一次测速
#[tauri::command]
pub async fn speedtest_run(state: State<'_, AppHandleState>) -> R<SpeedtestStatusDto> {
    let s = st(&state);
    let engine = s.require_speedtest().map_err(IpcError::from)?;

    if engine.is_running() {
        return Err(IpcError::from(AppError::new(ErrorCode::SpeedtestRunning)));
    }

    s.emit("speedtest:started", serde_json::json!({}));
    let run = engine
        .run(crate::domain::model::SpeedtestTrigger::Manual)
        .await
        .map_err(IpcError::from)?;

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
pub async fn speedtest_status(state: State<'_, AppHandleState>) -> R<SpeedtestStatusDto> {
    let s = st(&state);
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

/// 依据最近测速结果立即执行一次自动切换判定
#[tauri::command]
pub async fn switch_auto_apply(state: State<'_, AppHandleState>) -> R<Vec<SwitchRecordDto>> {
    let s = Arc::clone(&state.0);

    // 关键：先取出 Arc 并丢弃所有 parking_lot 守卫，再进入 await。
    // `parking_lot::RwLockReadGuard` 不是 Send，跨 await 会让整个 future 不 Send，
    // Tauri 命令必须 Send（在多线程 runtime 上跑）。
    let engine = {
        let guard = s.speedtest.read();
        guard.clone()
    }
    .ok_or_else(|| IpcError::from(AppError::new(ErrorCode::CoreNotRunning)))?;

    let settings = s.settings.get();

    let records = engine
        .auto_switch(
            &settings.switch_policy,
            &settings.speedtest_probe_url,
            settings.speedtest_timeout_ms,
        )
        .await
        .map_err(IpcError::from)?;

    let dtos: Vec<SwitchRecordDto> = records.iter().map(SwitchRecordDto::from).collect();
    if !dtos.is_empty() {
        s.emit(
            "switch:applied",
            serde_json::to_value(&dtos).unwrap_or_default(),
        );
    }
    Ok(dtos)
}

/// 手动切换历史
#[tauri::command]
pub async fn switch_history(state: State<'_, AppHandleState>, limit: Option<usize>) -> R<Vec<SwitchRecordDto>> {
    let s = st(&state);
    Ok(s.history
        .recent(limit.unwrap_or(50).min(200))
        .iter()
        .map(SwitchRecordDto::from)
        .collect())
}

/// 直接指定节点并记录为手动切换（供托盘快捷切换）
#[tauri::command]
pub async fn switch_manual(
    state: State<'_, AppHandleState>,
    group: String,
    node: String,
) -> R<SwitchRecordDto> {
    let s = st(&state);
    let api = s.require_api().map_err(IpcError::from)?;

    let info = api.proxy_group(&group).await.map_err(|e| match e {
        crate::domain::model::ApiError::NotFound(_) => {
            IpcError::from(AppError::new(ErrorCode::SwitchGroupMissing))
        }
        other => IpcError::from(AppError::from(other)),
    })?;

    let from = info.now.clone();
    api.set_group_selected(&group, &node)
        .await
        .map_err(|e| IpcError::from(AppError::from(e)))?;

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

/// 只读：当前组的 API 原始 kind（排障时确认是否为 Selector）
#[tauri::command]
pub async fn node_group_raw_kind(state: State<'_, AppHandleState>, group: String) -> R<String> {
    let s = st(&state);
    let api = s.require_api().map_err(IpcError::from)?;
    let info = api
        .proxy_group(&group)
        .await
        .map_err(|e| IpcError::from(AppError::from(e)))?;
    Ok(info.kind)
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
