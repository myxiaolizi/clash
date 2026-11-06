// ============================================================================
//  命令分发实现
// ============================================================================
//
// 这一层刻意保持「薄」：只做参数解包与错误包装，业务全在 AppState 里。
// 命名与参数逐字对齐 C.6 契约，因此 C# 侧可直接照契约表调用。

/// 从 args 里取必填字符串
fn req_str(args: &Value, key: &str) -> AppResult<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::msg(ErrorCode::Internal, format!("缺少参数 {key}")))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

async fn call(state: Arc<AppState>, cmd: &str, args: Value) -> AppResult<Value> {
    use crate::domain::errors::AppError;

    let to_val = |v: serde_json::Value| -> AppResult<Value> { Ok(v) };

    match cmd {
        // ==================== app ====================
        "app_info" => {
            let d = crate::commands::app_cmd::app_info_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "app_accept_compliance" => {
            let d = crate::commands::app_cmd::app_accept_compliance_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "app_open_data_dir" | "app_open_log_dir" => {
            let dir = if cmd == "app_open_data_dir" {
                state.paths.roaming.clone()
            } else {
                state.paths.logs_dir()
            };
            crate::commands::app_cmd::open_in_explorer(&dir.to_string_lossy())?;
            to_val(json!({"ok": true, "message": "已打开"}))
        }

        // ==================== settings ====================
        "settings_get" => {
            // 必须走 SettingsDto：直接序列化 AppSettings 会得到 snake_case 原始模型，
            // C# 侧按 camelCase 绑定会全部落空（曾经踩过）。
            let d = crate::commands::app_cmd::settings_get_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "settings_update" => {
            let patch: crate::commands::dto::SettingsPatch = serde_json::from_value(args)
                .map_err(|e| AppError::with_detail(ErrorCode::Internal, format!("参数不合法: {e}")))?;
            let d = crate::commands::app_cmd::settings_update_inner(&state, patch).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== core ====================
        "core_status" => {
            let d = crate::commands::app_cmd::core_status_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "core_start" => {
            crate::commands::app_cmd::core_start_inner(&state).await?;
            to_val(json!({"ok": true}))
        }
        "core_stop" => {
            crate::commands::app_cmd::core_stop_inner(&state).await?;
            to_val(json!({"ok": true}))
        }
        "core_restart" => {
            crate::commands::app_cmd::core_restart_inner(&state).await?;
            to_val(json!({"ok": true}))
        }
        "core_list_versions" => {
            let d = crate::commands::app_cmd::core_list_versions_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "core_install" => {
            let ver = opt_str(&args, "version");
            let d = crate::commands::app_cmd::core_install_inner(&state, ver).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== sysproxy ====================
        "sysproxy_get" => {
            let d = crate::commands::app_cmd::sysproxy_get_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "sysproxy_enable" => {
            crate::commands::app_cmd::sysproxy_enable_inner(&state).await?;
            to_val(json!({"ok": true}))
        }
        "sysproxy_disable" => {
            crate::commands::app_cmd::sysproxy_disable_inner(&state).await?;
            to_val(json!({"ok": true}))
        }

        // ==================== runtime ====================
        "runtime_get" => {
            let d = crate::commands::app_cmd::runtime_get_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== subscription ====================
        "subscription_list" => {
            let d = crate::commands::sub_cmd::subscription_list_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_add" => {
            let name = req_str(&args, "name")?;
            let url = req_str(&args, "url")?;
            let d = crate::commands::sub_cmd::subscription_add_inner(&state, name, url).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_add_ex" => {
            let name = req_str(&args, "name")?;
            let url = req_str(&args, "url")?;
            let username = opt_str(&args, "username");
            let password = opt_str(&args, "password");
            let homepage = opt_str(&args, "homepage");
            let remark = opt_str(&args, "remark");
            let d = crate::commands::sub_cmd::subscription_add_ex_inner(
                &state, name, url, username, password, homepage, remark,
            )
            .await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_remove" => {
            let uid = req_str(&args, "uid")?;
            crate::commands::sub_cmd::subscription_remove_inner(&state, uid).await?;
            to_val(json!({"ok": true}))
        }
        "subscription_update" => {
            let uid = req_str(&args, "uid")?;
            let name = opt_str(&args, "name");
            let enabled = args.get("enabled").and_then(|v| v.as_bool());
            let interval_min = args.get("intervalMin").and_then(|v| v.as_u64()).map(|v| v as u32);
            let d = crate::commands::sub_cmd::subscription_update_inner(&state, uid, name, enabled, interval_min).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_update_meta" => {
            let uid = req_str(&args, "uid")?;
            let username = opt_str(&args, "username");
            let password = opt_str(&args, "password");
            let homepage = opt_str(&args, "homepage");
            let remark = opt_str(&args, "remark");
            let d = crate::commands::sub_cmd::subscription_update_meta_inner(
                &state, uid, username, password, homepage, remark,
            )
            .await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_reveal_password" => {
            let uid = req_str(&args, "uid")?;
            let d = crate::commands::sub_cmd::subscription_reveal_password_inner(&state, uid).await?;
            to_val(Value::String(d))
        }
        "subscription_fetch" => {
            let uid = req_str(&args, "uid")?;
            let force = args.get("force").and_then(|v| v.as_bool());
            let d = crate::commands::sub_cmd::subscription_fetch_inner(&state, uid, force).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }        "subscription_fetch_all" => {
            let d = crate::commands::sub_cmd::subscription_fetch_all_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_traffic" => {
            let d = crate::commands::sub_cmd::subscription_traffic_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_apply" => {
            let d = crate::commands::sub_cmd::subscription_apply_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "subscription_preview_config" => {
            let d = crate::commands::sub_cmd::subscription_preview_config_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== rule（v3）====================
        "rule_list" => {
            let d = crate::commands::rule_cmd::rule_list_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "rule_add" => {
            let kind = req_str(&args, "kind")?;
            let value = req_str(&args, "value")?;
            let policy = req_str(&args, "policy")?;
            let enabled = args.get("enabled").and_then(|v| v.as_bool());
            let d = crate::commands::rule_cmd::rule_add_inner(&state, kind, value, policy, enabled).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "rule_update" => {
            let id = req_str(&args, "id")?;
            let kind = opt_str(&args, "kind");
            let value = opt_str(&args, "value");
            let policy = opt_str(&args, "policy");
            let enabled = args.get("enabled").and_then(|v| v.as_bool());
            let d = crate::commands::rule_cmd::rule_update_inner(&state, id, kind, value, policy, enabled).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "rule_remove" => {
            let id = req_str(&args, "id")?;
            let d = crate::commands::rule_cmd::rule_remove_inner(&state, id).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "rule_reorder" => {
            let ids: Vec<String> = args
                .get("ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .ok_or_else(|| AppError::msg(ErrorCode::Internal, "缺少参数 ids"))?;
            let d = crate::commands::rule_cmd::rule_reorder_inner(&state, ids).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== traffic / mode（v3）====================
        "traffic_stats" => {
            let d = crate::commands::rule_cmd::traffic_stats_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "traffic_reset_today" => {
            let d = crate::commands::rule_cmd::traffic_reset_today_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "mode_get" => {
            let d = crate::commands::rule_cmd::mode_get_inner(&state).await?;
            to_val(Value::String(d))
        }
        "mode_set" => {
            let mode = req_str(&args, "mode")?;
            let d = crate::commands::rule_cmd::mode_set_inner(&state, mode).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== node ====================
        "node_groups" => {
            let d = crate::commands::node_cmd::node_groups_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "node_select" => {
            let group = req_str(&args, "group")?;
            let node = req_str(&args, "node")?;
            let d = crate::commands::node_cmd::node_select_inner(&state, group, node).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "node_group_raw_kind" => {
            let group = req_str(&args, "group")?;
            let d = crate::commands::node_cmd::node_group_raw_kind_inner(&state, group).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== speedtest ====================
        "speedtest_run" => {
            let d = crate::commands::node_cmd::speedtest_run_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "speedtest_status" => {
            let d = crate::commands::node_cmd::speedtest_status_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== switch ====================
        "switch_auto_apply" => {
            let d = crate::commands::node_cmd::switch_auto_apply_inner(&state).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "switch_history" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);
            let d = crate::commands::node_cmd::switch_history_inner(&state, limit).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }
        "switch_manual" => {
            let group = req_str(&args, "group")?;
            let node = req_str(&args, "node")?;
            let d = crate::commands::node_cmd::switch_manual_inner(&state, group, node).await?;
            to_val(serde_json::to_value(d).map_err(|e| AppError::with_detail(ErrorCode::Internal, e))?)
        }

        // ==================== window（daemon 不涉及，返回占位）====================
        "runtime_show_window" | "window_minimize" | "window_toggle_maximize" | "window_close" => {
            Ok(json!({"ok": true, "message": "由 UI 进程处理"}))
        }

        other => Err(AppError::msg(ErrorCode::Internal, format!("未知命令: {other}"))),
    }
}
