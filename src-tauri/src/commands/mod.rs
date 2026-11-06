//! IPC 命令层（薄，只做参数校验与 DTO 转换）。

pub mod app_cmd;
pub mod dto;
pub mod node_cmd;
pub mod state;
pub mod sub_cmd;

pub use dto::IpcError;
pub use state::{AppEvent, AppHandleState, AppState};

/// 注册全部 IPC 命令（单一真源，新增命令只改这里）
pub fn register_handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        // ---- app ----
        app_cmd::app_info,
        app_cmd::app_accept_compliance,
        app_cmd::app_open_data_dir,
        app_cmd::app_open_log_dir,
        // ---- settings ----
        app_cmd::settings_get,
        app_cmd::settings_update,
        // ---- core ----
        app_cmd::core_status,
        app_cmd::core_start,
        app_cmd::core_stop,
        app_cmd::core_restart,
        app_cmd::core_list_versions,
        app_cmd::core_install,
        // ---- sysproxy / runtime ----
        app_cmd::sysproxy_get,
        app_cmd::sysproxy_enable,
        app_cmd::sysproxy_disable,
        app_cmd::runtime_get,
        app_cmd::runtime_show_window,
        // ---- window ----
        app_cmd::window_minimize,
        app_cmd::window_toggle_maximize,
        app_cmd::window_close,
        // ---- subscription ----
        sub_cmd::subscription_list,
        sub_cmd::subscription_add,
        sub_cmd::subscription_remove,
        sub_cmd::subscription_update,
        sub_cmd::subscription_fetch,
        sub_cmd::subscription_fetch_all,
        sub_cmd::subscription_traffic,
        sub_cmd::subscription_apply,
        sub_cmd::subscription_preview_config,
        // ---- node / speedtest / switch ----
        node_cmd::node_groups,
        node_cmd::node_select,
        node_cmd::node_group_raw_kind,
        node_cmd::speedtest_run,
        node_cmd::speedtest_status,
        node_cmd::switch_auto_apply,
        node_cmd::switch_history,
        node_cmd::switch_manual,
    ]
}
