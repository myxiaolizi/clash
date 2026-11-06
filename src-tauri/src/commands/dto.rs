//! 前端 DTO（camelCase）。内部模型 snake_case，跨 IPC 边界统一转 camelCase。

use serde::{Deserialize, Serialize};

use crate::domain::errors::AppError;
use crate::domain::model::{
    AppSettings, ProbeResult, Subscription, SubscriptionTraffic, SwitchPolicy, SwitchRecord,
    TrafficSummary, UserInfo,
};

/// 统一错误载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorDto {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub retryable: bool,
}

impl From<AppError> for ErrorDto {
    fn from(e: AppError) -> Self {
        Self {
            code: e.code.as_str().to_string(),
            message: e.message,
            detail: e.detail,
            retryable: e.retryable,
        }
    }
}

/// Tauri 要求 Err 类型实现 Serialize；用这个包装避免孤儿规则冲突
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct IpcError(pub ErrorDto);

impl From<AppError> for IpcError {
    fn from(e: AppError) -> Self {
        IpcError(e.into())
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.message)
    }
}

// ============================================================================
//  订阅
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionDto {
    pub uid: String,
    pub name: String,
    /// 脱敏后的地址提示（绝不含 token）
    pub url_hint: String,
    pub enabled: bool,
    pub update_interval_min: u32,
    pub last_update_at: Option<i64>,
    /// "never" | "ok" | "failed"
    pub last_update_status: String,
    pub last_error: Option<String>,
    pub node_count: u32,
    pub sort_order: u32,
    pub traffic: Option<TrafficDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficDto {
    pub upload: Option<i64>,
    pub download: Option<i64>,
    pub total: Option<i64>,
    pub used: Option<i64>,
    pub used_percent: Option<f32>,
    pub expire_at: Option<i64>,
    pub days_left: Option<i64>,
    /// "header" | "web" | "unknown"
    pub source: String,
    pub stale: bool,
}

impl From<&UserInfo> for TrafficDto {
    fn from(u: &UserInfo) -> Self {
        let now = crate::util::time::now_secs();
        Self {
            upload: u.upload,
            download: u.download,
            total: u.total,
            used: u.used(),
            used_percent: u.used_percent(),
            expire_at: u.expire_at,
            days_left: u.days_left(now),
            source: match u.source {
                crate::domain::model::UserInfoSource::Header => "header",
                crate::domain::model::UserInfoSource::Web => "web",
                crate::domain::model::UserInfoSource::Unknown => "unknown",
            }
            .to_string(),
            stale: u.expire_at.is_some_and(|e| e < now),
        }
    }
}

impl From<&Subscription> for SubscriptionDto {
    fn from(s: &Subscription) -> Self {
        let now = crate::util::time::now_secs();
        Self {
            uid: s.uid.clone(),
            name: s.name.clone(),
            url_hint: s.url_hint.clone(),
            enabled: s.enabled,
            update_interval_min: s.update_interval_min,
            last_update_at: s.last_update_at,
            last_update_status: match s.last_update_status {
                crate::domain::model::UpdateStatus::Never => "never",
                crate::domain::model::UpdateStatus::Ok => "ok",
                crate::domain::model::UpdateStatus::Failed => "failed",
            }
            .to_string(),
            last_error: s.last_error.clone(),
            node_count: s.node_count,
            sort_order: s.sort_order,
            traffic: s.mgmt.as_ref().map(|m| {
                let mut d = TrafficDto::from(m);
                d.stale = s.userinfo_stale(now);
                d
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficSummaryDto {
    pub items: Vec<ItemTrafficDto>,
    pub upload: i64,
    pub download: i64,
    pub total: Option<i64>,
    pub used: i64,
    pub used_percent: Option<f32>,
    pub expire_at: Option<i64>,
    pub days_left: Option<i64>,
    pub subscription_count: u32,
    pub has_missing: bool,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemTrafficDto {
    pub uid: String,
    pub name: String,
    pub upload: Option<i64>,
    pub download: Option<i64>,
    pub total: Option<i64>,
    pub used: Option<i64>,
    pub used_percent: Option<f32>,
    pub expire_at: Option<i64>,
    pub days_left: Option<i64>,
    pub source: String,
    pub stale: bool,
}

impl From<&TrafficSummary> for TrafficSummaryDto {
    fn from(t: &TrafficSummary) -> Self {
        Self {
            items: t
                .items
                .iter()
                .map(|i| ItemTrafficDto {
                    uid: i.uid.clone(),
                    name: i.name.clone(),
                    upload: i.upload,
                    download: i.download,
                    total: i.total,
                    used: i.used,
                    used_percent: i.used_percent,
                    expire_at: i.expire_at,
                    days_left: i.days_left,
                    source: src_str(&i.source),
                    stale: i.stale,
                })
                .collect(),
            upload: t.aggregate.upload,
            download: t.aggregate.download,
            total: t.aggregate.total,
            used: t.aggregate.used,
            used_percent: t.aggregate.used_percent,
            expire_at: t.aggregate.expire_at,
            days_left: t.aggregate.expire_at.map(|e| (e - t.at).div_euclid(86_400)),
            subscription_count: t.aggregate.subscription_count,
            has_missing: t.aggregate.has_missing,
            at: t.at,
        }
    }
}

fn src_str(s: &crate::domain::model::UserInfoSource) -> String {
    match s {
        crate::domain::model::UserInfoSource::Header => "header",
        crate::domain::model::UserInfoSource::Web => "web",
        crate::domain::model::UserInfoSource::Unknown => "unknown",
    }
    .to_string()
}

impl From<&SubscriptionTraffic> for ItemTrafficDto {
    fn from(i: &SubscriptionTraffic) -> Self {
        Self {
            uid: i.uid.clone(),
            name: i.name.clone(),
            upload: i.upload,
            download: i.download,
            total: i.total,
            used: i.used,
            used_percent: i.used_percent,
            expire_at: i.expire_at,
            days_left: i.days_left,
            source: src_str(&i.source),
            stale: i.stale,
        }
    }
}

// ============================================================================
//  设置
// ============================================================================

/// 设置 DTO：全部 camelCase，字段扁平便于前端绑定
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub locale: String,
    pub theme: String,
    pub auto_launch: bool,
    pub silent_start: bool,
    pub start_core_on_launch: bool,
    pub compliance_accepted: bool,
    pub proxy_mode: String,
    pub api_port: u16,
    pub core_version: Option<String>,
    pub core_download_proxy: Option<String>,
    pub sub_default_interval_min: u32,
    pub sub_fetch_concurrency: u8,
    pub sub_fetch_timeout_ms: u32,
    pub sub_fetch_proxy: Option<String>,
    pub sub_user_agent: String,
    pub speedtest_enabled: bool,
    pub speedtest_interval_min: u32,
    pub speedtest_probe_url: String,
    pub speedtest_timeout_ms: u32,
    pub speedtest_concurrency: u8,
    pub switch_policy: PolicyDto,
    pub notify_on_switch: bool,
    pub notify_on_expire: bool,
    pub traffic_warn_percent: u8,
    pub log_level: String,
    pub close_to_tray: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyDto {
    pub enabled: bool,
    pub tolerance_ms: u32,
    pub min_dwell_sec: u32,
    pub failure_threshold: u8,
    pub manual_override_min: u32,
    pub exclude_keywords: Vec<String>,
    pub prefer_region_priority: Vec<String>,
}

impl From<&SwitchPolicy> for PolicyDto {
    fn from(p: &SwitchPolicy) -> Self {
        Self {
            enabled: p.enabled,
            tolerance_ms: p.tolerance_ms,
            min_dwell_sec: p.min_dwell_sec,
            failure_threshold: p.failure_threshold,
            manual_override_min: p.manual_override_min,
            exclude_keywords: p.exclude_keywords.clone(),
            prefer_region_priority: p.prefer_region_priority.clone(),
        }
    }
}

impl From<&AppSettings> for SettingsDto {
    fn from(s: &AppSettings) -> Self {
        Self {
            locale: s.locale.clone(),
            theme: match s.theme {
                crate::domain::model::Theme::System => "system",
                crate::domain::model::Theme::Light => "light",
                crate::domain::model::Theme::Dark => "dark",
            }
            .to_string(),
            auto_launch: s.auto_launch,
            silent_start: s.silent_start,
            start_core_on_launch: s.start_core_on_launch,
            compliance_accepted: s.compliance_accepted_at.is_some(),
            proxy_mode: match s.proxy_mode {
                crate::domain::model::ProxyMode::SystemProxy => "system_proxy",
                crate::domain::model::ProxyMode::Tun => "tun",
                crate::domain::model::ProxyMode::Manual => "manual",
            }
            .to_string(),
            api_port: s.api_port,
            core_version: s.core_version.clone(),
            core_download_proxy: s.core_download_proxy.clone(),
            sub_default_interval_min: s.sub_default_interval_min,
            sub_fetch_concurrency: s.sub_fetch_concurrency,
            sub_fetch_timeout_ms: s.sub_fetch_timeout_ms,
            sub_fetch_proxy: s.sub_fetch_proxy.clone(),
            sub_user_agent: s.sub_user_agent.clone(),
            speedtest_enabled: s.speedtest_enabled,
            speedtest_interval_min: s.speedtest_interval_min,
            speedtest_probe_url: s.speedtest_probe_url.clone(),
            speedtest_timeout_ms: s.speedtest_timeout_ms,
            speedtest_concurrency: s.speedtest_concurrency,
            switch_policy: PolicyDto::from(&s.switch_policy),
            notify_on_switch: s.notify_on_switch,
            notify_on_expire: s.notify_on_expire,
            traffic_warn_percent: s.traffic_warn_percent,
            log_level: match s.log_level {
                crate::domain::model::LogLevel::Error => "error",
                crate::domain::model::LogLevel::Warn => "warn",
                crate::domain::model::LogLevel::Info => "info",
                crate::domain::model::LogLevel::Debug => "debug",
            }
            .to_string(),
            close_to_tray: s.close_to_tray,
        }
    }
}

/// 设置补丁：全部 Option，None 表示不修改
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub locale: Option<String>,
    pub theme: Option<String>,
    pub auto_launch: Option<bool>,
    pub silent_start: Option<bool>,
    pub start_core_on_launch: Option<bool>,
    pub proxy_mode: Option<String>,
    pub api_port: Option<u16>,
    pub core_version: Option<String>,
    pub core_download_proxy: Option<String>,
    pub sub_default_interval_min: Option<u32>,
    pub sub_fetch_concurrency: Option<u8>,
    pub sub_fetch_timeout_ms: Option<u32>,
    pub sub_fetch_proxy: Option<String>,
    pub sub_user_agent: Option<String>,
    pub speedtest_enabled: Option<bool>,
    pub speedtest_interval_min: Option<u32>,
    pub speedtest_probe_url: Option<String>,
    pub speedtest_timeout_ms: Option<u32>,
    pub speedtest_concurrency: Option<u8>,
    pub switch_policy: Option<PolicyDto>,
    pub notify_on_switch: Option<bool>,
    pub notify_on_expire: Option<bool>,
    pub traffic_warn_percent: Option<u8>,
    pub log_level: Option<String>,
    pub close_to_tray: Option<bool>,
}

// ============================================================================
//  运行态 / 内核
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreStatusDto {
    /// "stopped" | "starting" | "running" | "stopping" | "failed"
    pub state: String,
    pub error: Option<String>,
    pub version: Option<String>,
    pub mixed_port: Option<u16>,
    pub api_port: Option<u16>,
    pub uptime_sec: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDto {
    pub core: CoreStatusDto,
    pub sysproxy_enabled: bool,
    pub sysproxy_managed: bool,
    pub sysproxy_server: Option<String>,
    pub active_subscription: Option<String>,
    pub traffic_up: u64,
    pub traffic_down: u64,
    pub up_rate: u64,
    pub down_rate: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledCoreDto {
    pub version: String,
    pub size: u64,
    pub installed_at: i64,
    pub is_active: bool,
}

// ============================================================================
//  节点 / 测速
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDto {
    pub name: String,
    pub delay_ms: Option<u32>,
    pub ok: bool,
    pub rank: Option<usize>,
    pub error: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    pub name: String,
    pub now: Option<String>,
    pub node_count: u32,
    pub nodes: Vec<NodeDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedtestStatusDto {
    pub running: bool,
    pub run_id: Option<String>,
    pub total: u32,
    pub finished: u32,
    pub ok_count: u32,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub best_delay: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchRecordDto {
    pub at: i64,
    pub group: String,
    pub from_node: Option<String>,
    pub to_node: String,
    pub from_delay: Option<u32>,
    pub to_delay: Option<u32>,
    pub reason: String,
}

impl From<&SwitchRecord> for SwitchRecordDto {
    fn from(r: &SwitchRecord) -> Self {
        Self {
            at: r.at,
            group: r.group.clone(),
            from_node: r.from_node.clone(),
            to_node: r.to_node.clone(),
            from_delay: r.from_delay,
            to_delay: r.to_delay,
            reason: match r.reason {
                crate::domain::model::SwitchReason::Best => "best",
                crate::domain::model::SwitchReason::CurrentFailed => "current_failed",
                crate::domain::model::SwitchReason::Manual => "manual",
                crate::domain::model::SwitchReason::GroupMissing => "group_missing",
            }
            .to_string(),
        }
    }
}

/// 前端可直接用 `ProbeResult` 的字段（已在 model 中定义 camelCase 由 serde 处理）
pub type ProbeDto = ProbeResult;

// ============================================================================
//  通用
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfoDto {
    pub version: String,
    pub core_version: Option<String>,
    pub data_dir: String,
    pub log_dir: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpResultDto {
    pub ok: bool,
    pub message: String,
}

impl OpResultDto {
    pub fn ok(msg: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: msg.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::AppSettings;

    #[test]
    fn settings_dto_is_camelcase() {
        let s = AppSettings::default();
        let dto = SettingsDto::from(&s);
        let j = serde_json::to_string(&dto).unwrap();
        assert!(j.contains("\"autoLaunch\""), "必须是 camelCase");
        assert!(!j.contains("\"auto_launch\""), "不得泄露 snake_case");
        assert!(j.contains("\"switchPolicy\""));
        assert!(j.contains("\"subDefaultIntervalMin\""));
    }

    #[test]
    fn policy_dto_roundtrip() {
        let p = SwitchPolicy::default();
        let dto = PolicyDto::from(&p);
        assert_eq!(dto.tolerance_ms, 30);
        assert_eq!(dto.min_dwell_sec, 300);
        assert_eq!(dto.failure_threshold, 2);
        assert_eq!(dto.manual_override_min, 60);
    }

    #[test]
    fn error_dto_uses_string_code() {
        let e = AppError::new(crate::domain::errors::ErrorCode::NetTimeout);
        let dto: ErrorDto = e.into();
        assert_eq!(dto.code, "E_NET_TIMEOUT");
        assert!(dto.retryable);
        assert!(dto.detail.is_none());
    }

    #[test]
    fn ipc_error_serializes_transparently() {
        let e = AppError::new(crate::domain::errors::ErrorCode::Busy);
        let ipc: IpcError = e.into();
        let j = serde_json::to_string(&ipc).unwrap();
        assert!(j.contains("\"E_BUSY\""), "不得多包一层: {j}");
        assert!(j.starts_with('{'));
    }

    #[test]
    fn subscription_dto_hides_cipher() {
        let sub = Subscription::new(
            "u1".into(),
            "机场".into(),
            "CIPHER_SECRET_VALUE".into(),
            "https://a.com/api/v1/******".into(),
            360,
        );
        let dto = SubscriptionDto::from(&sub);
        let j = serde_json::to_string(&dto).unwrap();
        assert!(!j.contains("CIPHER_SECRET_VALUE"), "绝不能把密文发给前端");
        assert!(!j.contains("url_cipher"));
        assert!(j.contains("urlHint"));
    }

    #[test]
    fn subscription_dto_status_strings() {
        let mut sub = Subscription::new("u".into(), "n".into(), "c".into(), "h".into(), 360);
        assert_eq!(SubscriptionDto::from(&sub).last_update_status, "never");
        sub.last_update_status = crate::domain::model::UpdateStatus::Ok;
        assert_eq!(SubscriptionDto::from(&sub).last_update_status, "ok");
        sub.last_update_status = crate::domain::model::UpdateStatus::Failed;
        assert_eq!(SubscriptionDto::from(&sub).last_update_status, "failed");
    }

    #[test]
    fn switch_record_dto_reason_strings() {
        let mk = |r: crate::domain::model::SwitchReason| SwitchRecord {
            at: 1,
            group: "Proxy".into(),
            from_node: None,
            to_node: "n".into(),
            from_delay: None,
            to_delay: None,
            reason: r,
        };
        use crate::domain::model::SwitchReason as SR;
        assert_eq!(SwitchRecordDto::from(&mk(SR::Best)).reason, "best");
        assert_eq!(
            SwitchRecordDto::from(&mk(SR::CurrentFailed)).reason,
            "current_failed"
        );
        assert_eq!(SwitchRecordDto::from(&mk(SR::Manual)).reason, "manual");
    }

    #[test]
    fn switch_record_preserves_none_delay() {
        let r = SwitchRecord {
            at: 1,
            group: "g".into(),
            from_node: Some("a".into()),
            to_node: "b".into(),
            from_delay: Some(100),
            to_delay: None, // 手动切换无测速数据
            reason: crate::domain::model::SwitchReason::Manual,
        };
        let dto = SwitchRecordDto::from(&r);
        assert_eq!(dto.to_delay, None, "不得用 0 冒充未知");
        let j = serde_json::to_string(&dto).unwrap();
        assert!(j.contains("\"toDelay\":null"));
    }

    #[test]
    fn traffic_summary_dto_flattens_aggregate() {
        let mut a = Subscription::new("a".into(), "A".into(), "c".into(), "h".into(), 360);
        a.mgmt = Some(UserInfo {
            upload: Some(100),
            download: Some(200),
            total: Some(1000),
            expire_at: Some(crate::util::time::now_secs() + 86_400),
            fetched_at: crate::util::time::now_secs(),
            source: crate::domain::model::UserInfoSource::Header,
        });
        let t = TrafficSummary::build(&[a], crate::util::time::now_secs());
        let dto = TrafficSummaryDto::from(&t);
        assert_eq!(dto.upload, 100);
        assert_eq!(dto.used, 300);
        assert_eq!(dto.total, Some(1000));
        assert_eq!(dto.days_left, Some(1));
        assert_eq!(dto.items.len(), 1);

        let j = serde_json::to_string(&dto).unwrap();
        assert!(j.contains("\"usedPercent\""));
        assert!(j.contains("\"daysLeft\""));
    }

    #[test]
    fn settings_patch_all_optional() {
        let p: SettingsPatch = serde_json::from_str("{}").unwrap();
        assert!(p.theme.is_none());
        assert!(p.auto_launch.is_none());
        // 只给一个字段也应能解析
        let p2: SettingsPatch =
            serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(p2.theme.as_deref(), Some("dark"));
    }
}
