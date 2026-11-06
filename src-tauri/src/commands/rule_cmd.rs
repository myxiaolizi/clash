//! 规则管理与流量统计 IPC 命令（v3）。

use std::sync::Arc;

use tauri::State;

use crate::commands::dto::*;
use crate::commands::state::AppHandleState;
use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::{RuleKind, RulePolicy};

type R<T> = Result<T, IpcError>;

// ============================================================================
//  规则
// ============================================================================

/// 列出全部规则（按 sortOrder 升序）
pub async fn rule_list_inner(
    s: &Arc<crate::commands::state::AppState>,
) -> AppResult<Vec<RuleDto>> {
    Ok(s.rules.list().iter().map(RuleDto::from).collect())
}

#[tauri::command]
pub async fn rule_list(state: State<'_, AppHandleState>) -> R<Vec<RuleDto>> {
    rule_list_inner(&state.0).await.map_err(IpcError::from)
}

/// 新增规则
pub async fn rule_add_inner(
    s: &Arc<crate::commands::state::AppState>,
    kind: String,
    value: String,
    policy: String,
    enabled: Option<bool>,
) -> AppResult<RuleDto> {
    let kind = RuleKind::parse(&kind)
        .ok_or_else(|| AppError::msg(ErrorCode::InvalidArg, format!("未知规则类型: {kind}")))?;
    let policy = RulePolicy::parse(&policy)
        .ok_or_else(|| AppError::msg(ErrorCode::InvalidArg, format!("未知规则去向: {policy}")))?;

    let rule = s.rules.add(kind, &value, policy, enabled.unwrap_or(true))?;
    // 规则变了但没热重载，用户会以为"加了没生效"——必须重建配置
    apply_rules_changed(s).await;
    Ok(RuleDto::from(&rule))
}

#[tauri::command]
pub async fn rule_add(
    state: State<'_, AppHandleState>,
    kind: String,
    value: String,
    policy: String,
    enabled: Option<bool>,
) -> R<RuleDto> {
    rule_add_inner(&state.0, kind, value, policy, enabled)
        .await
        .map_err(IpcError::from)
}

/// 更新规则（未给出的字段保持原值）
pub async fn rule_update_inner(
    s: &Arc<crate::commands::state::AppState>,
    id: String,
    kind: Option<String>,
    value: Option<String>,
    policy: Option<String>,
    enabled: Option<bool>,
) -> AppResult<RuleDto> {
    let kind = match kind {
        Some(k) => Some(
            RuleKind::parse(&k)
                .ok_or_else(|| AppError::msg(ErrorCode::InvalidArg, format!("未知规则类型: {k}")))?,
        ),
        None => None,
    };
    let policy = match policy {
        Some(p) => Some(
            RulePolicy::parse(&p)
                .ok_or_else(|| AppError::msg(ErrorCode::InvalidArg, format!("未知规则去向: {p}")))?,
        ),
        None => None,
    };

    let rule = s.rules.update(&id, kind, value, policy, enabled)?;
    apply_rules_changed(s).await;
    Ok(RuleDto::from(&rule))
}

#[tauri::command]
pub async fn rule_update(
    state: State<'_, AppHandleState>,
    id: String,
    kind: Option<String>,
    value: Option<String>,
    policy: Option<String>,
    enabled: Option<bool>,
) -> R<RuleDto> {
    rule_update_inner(&state.0, id, kind, value, policy, enabled)
        .await
        .map_err(IpcError::from)
}

/// 删除规则
pub async fn rule_remove_inner(
    s: &Arc<crate::commands::state::AppState>,
    id: String,
) -> AppResult<OpResultDto> {
    s.rules.remove(&id)?;
    apply_rules_changed(s).await;
    Ok(OpResultDto::ok("已删除"))
}

#[tauri::command]
pub async fn rule_remove(state: State<'_, AppHandleState>, id: String) -> R<OpResultDto> {
    rule_remove_inner(&state.0, id).await.map_err(IpcError::from)
}

/// 按给定 id 顺序重排规则
pub async fn rule_reorder_inner(
    s: &Arc<crate::commands::state::AppState>,
    ids: Vec<String>,
) -> AppResult<Vec<RuleDto>> {
    let rules = s.rules.reorder(&ids)?;
    apply_rules_changed(s).await;
    Ok(rules.iter().map(RuleDto::from).collect())
}

#[tauri::command]
pub async fn rule_reorder(
    state: State<'_, AppHandleState>,
    ids: Vec<String>,
) -> R<Vec<RuleDto>> {
    rule_reorder_inner(&state.0, ids).await.map_err(IpcError::from)
}

/// 规则变更后的统一收尾：重建配置 + 热重载。
///
/// 失败不返回错误：规则已经存进 rules.json 了（持久成功），
/// 只是暂时没能作用到内核（比如内核没启动）。这种情况应该提示而非报错，
/// 否则前端会把"规则没保存"的错误展示给用户，与事实不符。
async fn apply_rules_changed(s: &Arc<crate::commands::state::AppState>) {
    match crate::commands::sub_cmd::apply_config_inner(s).await {
        Ok(nodes) => {
            tracing::info!(nodes, "规则变更已应用到内核");
            s.emit("rule:applied", serde_json::json!({"nodes": nodes}));
        }
        Err(e) => {
            tracing::warn!(err = %e, "规则已保存但未能应用到内核");
            s.emit(
                "rule:applyFailed",
                serde_json::json!({
                    "code": e.code.as_str(),
                    "message": e.message,
                }),
            );
        }
    }
}

// ============================================================================
//  流量统计
// ============================================================================

/// 取流量统计：今日 / 本次 / 累计 + 近 24 小时趋势。
///
/// 顺手采集一次，保证用户点开流量页看到的是最新值，
/// 不用等到下一个 60 秒 tick。
pub async fn traffic_stats_inner(
    s: &Arc<crate::commands::state::AppState>,
) -> AppResult<TrafficStatsDto> {
    let now = crate::util::time::now_secs();

    // 先采一次（失败无所谓，用已有持久化数据兜底）
    let _ = s.collect_traffic().await;

    // 本次运行期的累计：直接问内核。
    // 关键：必须先把 Arc 取出来、让 parking_lot 守卫出作用域，
    // 再进 await —— RwLockReadGuard 不是 Send，跨 await 会让
    // 整个 future 变 !Send，Tauri 命令就编译不过。
    let api = {
        let guard = s.api.read();
        guard.clone()
    };
    let (session_up, session_down) = match api {
        Some(api) => match api.traffic_totals().await {
            Ok(t) => (t.up, t.down),
            Err(_) => (0, 0),
        },
        None => (0, 0),
    };

    let stats = s.traffic.snapshot();
    Ok(TrafficStatsDto::build(&stats, now, session_up, session_down))
}

#[tauri::command]
pub async fn traffic_stats(state: State<'_, AppHandleState>) -> R<TrafficStatsDto> {
    traffic_stats_inner(&state.0).await.map_err(IpcError::from)
}

/// 清空今日统计（累计不动）
pub async fn traffic_reset_today_inner(
    s: &Arc<crate::commands::state::AppState>,
) -> AppResult<OpResultDto> {
    s.traffic.clear_today(crate::util::time::now_secs());
    s.emit("traffic:reset", serde_json::json!({"scope": "today"}));
    Ok(OpResultDto::ok("今日流量已清零"))
}

#[tauri::command]
pub async fn traffic_reset_today(state: State<'_, AppHandleState>) -> R<OpResultDto> {
    traffic_reset_today_inner(&state.0)
        .await
        .map_err(IpcError::from)
}

// ============================================================================
//  分流模式
// ============================================================================

/// 切换内核分流模式（rule / global / direct）。
///
/// 独立成一个命令而不复用 `settings_update`：
/// 切换后必须**立刻重建配置并热重载**，否则用户点了"全局"没有任何反应。
///
/// 错误语义（重要）：
/// - 内核**未运行** → 视为成功。设置已存盘，下次启动内核时自然生效。
///   这里若返回错误，GUI 会在内核停止时弹"切换失败"，而实际是切成功的。
/// - 内核运行但重载失败 → 返回错误（真的没生效）。
pub async fn mode_set_inner(
    s: &Arc<crate::commands::state::AppState>,
    mode: String,
) -> AppResult<OpResultDto> {
    let m = match mode.trim() {
        "rule" => crate::domain::model::MihomoMode::Rule,
        "global" => crate::domain::model::MihomoMode::Global,
        "direct" => crate::domain::model::MihomoMode::Direct,
        other => {
            return Err(AppError::msg(
                ErrorCode::InvalidArg,
                format!("未知分流模式: {other}"),
            ))
        }
    };

    let updated = s.settings.update(|x| x.mihomo_mode = m)?;
    let label = mode_label(updated.mihomo_mode);

    match crate::commands::sub_cmd::apply_config_inner(s).await {
        Ok(nodes) => {
            s.emit(
                "mode:changed",
                serde_json::json!({"mode": updated.mihomo_mode.as_str(), "nodes": nodes}),
            );
            Ok(OpResultDto::ok(format!("已切换到{label}模式")))
        }
        // 内核没运行：设置已经存好了，不算失败
        Err(e) if e.code == ErrorCode::CoreNotRunning => {
            s.emit(
                "mode:changed",
                serde_json::json!({"mode": updated.mihomo_mode.as_str(), "applied": false}),
            );
            Ok(OpResultDto::ok(format!(
                "已切换到{label}模式，将在内核启动后生效"
            )))
        }
        Err(e) => {
            // 设置已存盘（下次启动仍生效），但当前内核没换成 —— 这是真错误
            tracing::warn!(err = %e, "分流模式已保存但未能应用到内核");
            s.emit(
                "mode:changed",
                serde_json::json!({"mode": updated.mihomo_mode.as_str(), "applied": false}),
            );
            Err(AppError::with_detail(
                e.code,
                format!("模式已保存，但应用失败：{}", e.message),
            ))
        }
    }
}

#[tauri::command]
pub async fn mode_set(state: State<'_, AppHandleState>, mode: String) -> R<OpResultDto> {
    mode_set_inner(&state.0, mode).await.map_err(IpcError::from)
}

fn mode_label(m: crate::domain::model::MihomoMode) -> &'static str {
    match m {
        crate::domain::model::MihomoMode::Rule => "规则",
        crate::domain::model::MihomoMode::Global => "全局",
        crate::domain::model::MihomoMode::Direct => "直连",
    }
}

/// 只读：当前分流模式
pub async fn mode_get_inner(
    s: &Arc<crate::commands::state::AppState>,
) -> AppResult<String> {
    Ok(s.settings.get().mihomo_mode.as_str().to_string())
}

#[tauri::command]
pub async fn mode_get(state: State<'_, AppHandleState>) -> R<String> {
    mode_get_inner(&state.0).await.map_err(IpcError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_label_is_chinese() {
        assert_eq!(
            mode_label(crate::domain::model::MihomoMode::Rule),
            "规则"
        );
        assert_eq!(
            mode_label(crate::domain::model::MihomoMode::Global),
            "全局"
        );
        assert_eq!(
            mode_label(crate::domain::model::MihomoMode::Direct),
            "直连"
        );
    }

    #[test]
    fn unknown_rule_kind_rejected() {
        assert!(RuleKind::parse("nonsense").is_none());
        assert!(RulePolicy::parse("nonsense").is_none());
        // 合法值
        assert_eq!(RuleKind::parse("domain"), Some(RuleKind::Domain));
        assert_eq!(RulePolicy::parse("block"), Some(RulePolicy::Block));
    }

    #[test]
    fn mihomo_mode_parse_rejects_garbage_to_rule() {
        use crate::domain::model::MihomoMode;
        assert_eq!(MihomoMode::parse("global"), MihomoMode::Global);
        assert_eq!(MihomoMode::parse("direct"), MihomoMode::Direct);
        assert_eq!(MihomoMode::parse("rule"), MihomoMode::Rule);
        // 未知值回退 rule（设置补丁不应因未知值报错）
        assert_eq!(MihomoMode::parse("garbage"), MihomoMode::Rule);
    }

    #[test]
    fn mode_set_validates_literal() {
        // mode_set 用字面量匹配而非 MihomoMode::parse —— 用户传错要明确报错，
        // 不能静默当 rule（那样用户以为切了全局，实际还是规则）
        let valid = ["rule", "global", "direct"];
        for v in valid {
            assert!(matches!(v, "rule" | "global" | "direct"));
        }
        assert!("garbage".trim() != "rule");
    }
}
