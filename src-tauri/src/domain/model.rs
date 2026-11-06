//! 纯数据模型（契约 C.5）。本模块**不含任何 IO**，可完整单测。
//!
//! 序列化约定：
//! - 内部持久化用 `snake_case`（与数据文件字段一致）
//! - 暴露给前端的 DTO 在 `commands/dto.rs` 中转为 `camelCase`

use serde::{Deserialize, Serialize};

// ============================================================================
//  C.5 设置
// ============================================================================

/// 当前设置 schema 版本。任何字段增删都必须升这个值，并在 migrate.rs 里处理。
///
/// v1 → v2：新增 `compliance_accepted_at`
/// v2 → v3：新增 `SwitchPolicy` 的失败转移字段；订阅新增用户名/密码/机场地址/备注；
///          新增 `mihomo_mode`（规则/全局分流模式，与 `proxy_mode` 正交）
pub const SETTINGS_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::System
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    SystemProxy,
    Tun,
    Manual,
}

impl Default for ProxyMode {
    fn default() -> Self {
        ProxyMode::SystemProxy
    }
}

/// 内核**分流模式**（mihomo `mode` 字段）。
///
/// 与 `ProxyMode` 是两个正交概念，早期实现混为一谈：
/// - `ProxyMode` 回答「流量怎么进内核」：系统代理 / TUN / 手动
/// - `MihomoMode` 回答「进内核后怎么走」：按规则 / 全局 / 直连
///
/// 前端的「规则 / 全局」开关操作的是本枚举，它**不改**抓流方式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MihomoMode {
    Rule,
    Global,
    Direct,
}

impl Default for MihomoMode {
    fn default() -> Self {
        MihomoMode::Rule
    }
}

impl MihomoMode {
    /// 写入 config.yaml 的 `mode` 值（内核只认这三个小写字面量）
    pub fn as_str(self) -> &'static str {
        match self {
            MihomoMode::Rule => "rule",
            MihomoMode::Global => "global",
            MihomoMode::Direct => "direct",
        }
    }

    /// 从设置补丁里的字符串解析；未知值回退 rule（不得报错打断 UI）
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "global" => MihomoMode::Global,
            "direct" => MihomoMode::Direct,
            _ => MihomoMode::Rule,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl Default for LogLevel {
    fn default() -> Self {
        LogLevel::Info
    }
}

impl LogLevel {
    /// 映射到 tracing 的 LevelFilter
    pub fn to_filter(self) -> tracing::level_filters::LevelFilter {
        use tracing::level_filters::LevelFilter;
        match self {
            LogLevel::Error => LevelFilter::ERROR,
            LogLevel::Warn => LevelFilter::WARN,
            LogLevel::Info => LevelFilter::INFO,
            LogLevel::Debug => LevelFilter::DEBUG,
        }
    }

    /// 映射到 mihomo 的 `log-level`（注意内核用 `warning` 不是 `warn`）
    pub fn to_mihomo(self) -> &'static str {
        match self {
            LogLevel::Error => "error",
            LogLevel::Warn => "warning",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
        }
    }
}

/// 切换策略（C.5）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SwitchPolicy {
    /// 是否启用自动切换
    pub enabled: bool,
    /// 作用范围
    pub scope: SwitchScope,
    /// 新节点须比当前快 ≥ 该值才切换（防抖）
    pub tolerance_ms: u32,
    /// 切换后最短驻留时间（秒）
    pub min_dwell_sec: u32,
    /// 连续 N 次探测失败才降级
    pub failure_threshold: u8,
    /// 用户手动选节点后，该组暂停自动切换的分钟数（0 = 不暂停）
    pub manual_override_min: u32,
    /// 排除关键词（信息节点、到期提醒等）
    pub exclude_keywords: Vec<String>,
    /// 地区优先级（为空则不参与排序）
    pub prefer_region_priority: Vec<String>,

    // ---- v3 新增：失败转移 ----
    /// 当前节点连续失败达阈值后的动作
    #[serde(default)]
    pub on_failure_action: OnFailureAction,
    /// 失败自动转移总开关（关掉后 on_failure_action 不生效）
    #[serde(default = "default_true")]
    pub failover_enabled: bool,
    /// 重试模式下两次探测之间的间隔（秒）
    #[serde(default = "default_retry_interval")]
    pub retry_interval_sec: u32,
    /// 重试模式下最多重试几次，超过则视为失败
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// 切换后额外冷却时间（秒），在 min_dwell 之后继续抑制回切
    #[serde(default)]
    pub dwell_after_switch_sec: u32,
    /// 只把本轮探测成功的节点当作候选（默认 true；关掉后允许用历史延迟兜底）
    #[serde(default = "default_true")]
    pub prefer_healthy_only: bool,
}

fn default_true() -> bool {
    true
}

fn default_retry_interval() -> u32 {
    5
}

fn default_max_retries() -> u32 {
    2
}

/// 失败达阈值后的处置方式
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnFailureAction {
    /// 切到下一个可用节点（默认）
    Switch,
    /// 原地重试当前节点，超过 max_retries 才切
    Retry,
    /// 什么都不做，只在历史里留痕
    Ignore,
}

impl Default for OnFailureAction {
    fn default() -> Self {
        OnFailureAction::Switch
    }
}

impl OnFailureAction {
    pub fn as_str(self) -> &'static str {
        match self {
            OnFailureAction::Switch => "switch",
            OnFailureAction::Retry => "retry",
            OnFailureAction::Ignore => "none",
        }
    }

    /// 从设置补丁解析；注意契约里「无动作」写作 `"none"`
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "retry" => OnFailureAction::Retry,
            "none" => OnFailureAction::Ignore,
            _ => OnFailureAction::Switch,
        }
    }
}

impl Default for SwitchPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: SwitchScope::Group("Proxy".to_string()),
            tolerance_ms: 30,
            min_dwell_sec: 300,
            failure_threshold: 2,
            manual_override_min: 60,
            exclude_keywords: vec![
                "到期".into(),
                "过期".into(),
                "剩余".into(),
                "流量".into(),
                "官网".into(),
                "套餐".into(),
                "重置".into(),
                "客服".into(),
                "域名".into(),
            ],
            prefer_region_priority: Vec::new(),
            on_failure_action: OnFailureAction::Switch,
            failover_enabled: true,
            retry_interval_sec: 5,
            max_retries: 2,
            dwell_after_switch_sec: 0,
            prefer_healthy_only: true,
        }
    }
}

impl SwitchPolicy {
    /// 运行时校验与归一（防止用户输入越界值破坏调度）
    pub fn sanitize(&mut self) {
        self.tolerance_ms = self.tolerance_ms.min(2000);
        self.min_dwell_sec = self.min_dwell_sec.clamp(30, 86_400);
        self.failure_threshold = self.failure_threshold.clamp(1, 10);
        self.manual_override_min = self.manual_override_min.min(1440);
        // 重试间隔不许为 0（会变成忙轮询打满内核 API）
        self.retry_interval_sec = self.retry_interval_sec.clamp(1, 300);
        self.max_retries = self.max_retries.min(10);
        // 额外冷却最多一天，且不得短于 0
        self.dwell_after_switch_sec = self.dwell_after_switch_sec.min(86_400);
        // 关键词去重去空
        let mut seen = std::collections::HashSet::new();
        self.exclude_keywords = self
            .exclude_keywords
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .filter(|s| seen.insert(s.clone()))
            .collect();
    }

    /// 切换后总共需要冷却多久才允许再次自动切换。
    ///
    /// 取 `min_dwell_sec + dwell_after_switch_sec`：前者是「别抖」，
    /// 后者是「刚切过别马上切回来」，语义独立但可以叠加。
    pub fn total_dwell_sec(&self) -> u32 {
        self.min_dwell_sec
            .saturating_add(self.dwell_after_switch_sec)
            .min(86_400)
    }

    /// 失败处置是否真的会做切换（Ignore / 关闭 failover 时都不会）
    pub fn should_failover(&self) -> bool {
        self.failover_enabled && self.on_failure_action != OnFailureAction::Ignore
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwitchScope {
    Global,
    Group(String),
}

/// 应用设置（C.5 `AppSettings`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub schema_version: u32,
    pub locale: String,
    pub theme: Theme,
    pub auto_launch: bool,
    pub silent_start: bool,
    pub start_core_on_launch: bool,
    /// 首次合规提示接受时间；None = 未接受，必须再弹
    pub compliance_accepted_at: Option<i64>,
    pub proxy_mode: ProxyMode,
    /// 内核分流模式：rule（按规则）/ global（全走主组）/ direct（全直连）。
    ///
    /// **与 `proxy_mode` 正交**：改这个不动系统代理/TUN。
    #[serde(default)]
    pub mihomo_mode: MihomoMode,
    /// 0 = 自动分配
    pub api_port: u16,
    /// 固定 "127.0.0.1"，不可由用户修改
    pub bind_address: String,
    pub core_version: Option<String>,
    /// 仅用于下载内核的代理
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
    pub switch_policy: SwitchPolicy,
    pub notify_on_switch: bool,
    pub notify_on_expire: bool,
    pub traffic_warn_percent: u8,
    pub log_level: LogLevel,
    pub close_to_tray: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            locale: "zh-CN".to_string(),
            theme: Theme::System,
            auto_launch: false,
            silent_start: true,
            start_core_on_launch: true,
            compliance_accepted_at: None,
            proxy_mode: ProxyMode::SystemProxy,
            mihomo_mode: MihomoMode::Rule,
            api_port: 0,
            bind_address: "127.0.0.1".to_string(),
            core_version: None,
            core_download_proxy: None,
            sub_default_interval_min: 360,
            sub_fetch_concurrency: 2,
            sub_fetch_timeout_ms: 15_000,
            sub_fetch_proxy: None,
            sub_user_agent: "clash.meta/1.19.0".to_string(),
            speedtest_enabled: true,
            speedtest_interval_min: 30,
            speedtest_probe_url: "http://www.gstatic.com/generate_204".to_string(),
            speedtest_timeout_ms: 3000,
            speedtest_concurrency: 8,
            switch_policy: SwitchPolicy::default(),
            notify_on_switch: false,
            notify_on_expire: true,
            traffic_warn_percent: 80,
            log_level: LogLevel::Info,
            close_to_tray: true,
        }
    }
}

impl AppSettings {
    /// 归一化到合法区间（C.6 周期下限、并发上限等硬约束）
    pub fn sanitize(&mut self) {
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        // 订阅周期下限 15 分钟
        self.sub_default_interval_min = self.sub_default_interval_min.max(15);
        self.sub_fetch_concurrency = self.sub_fetch_concurrency.clamp(1, 4);
        self.sub_fetch_timeout_ms = self.sub_fetch_timeout_ms.clamp(3_000, 120_000);
        // 测速周期下限 5 分钟
        self.speedtest_interval_min = self.speedtest_interval_min.max(5);
        self.speedtest_concurrency = self.speedtest_concurrency.clamp(1, 16);
        self.speedtest_timeout_ms = self.speedtest_timeout_ms.clamp(1_000, 10_000);
        self.traffic_warn_percent = self.traffic_warn_percent.min(100);
        // 绑定地址不可改
        self.bind_address = "127.0.0.1".to_string();
        if self.speedtest_probe_url.trim().is_empty() {
            self.speedtest_probe_url = "http://www.gstatic.com/generate_204".to_string();
        }
        if self.sub_user_agent.trim().is_empty() {
            self.sub_user_agent = "clash.meta/1.19.0".to_string();
        }
        self.switch_policy.sanitize();
    }
}

// ============================================================================
//  C.5 订阅
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Never,
    Ok,
    Failed,
}

impl Default for UpdateStatus {
    fn default() -> Self {
        UpdateStatus::Never
    }
}

/// 机场返回的流量信息（来自 `subscription-userinfo` 头）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UserInfo {
    pub upload: Option<i64>,
    pub download: Option<i64>,
    /// 0 表示不限量/未知
    pub total: Option<i64>,
    /// Unix 秒；None 表示无限期
    pub expire_at: Option<i64>,
    pub fetched_at: i64,
    pub source: UserInfoSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserInfoSource {
    /// 来自响应头
    Header,
    /// 来自网页兜底
    Web,
    /// 无法获取
    Unknown,
}

impl Default for UserInfoSource {
    fn default() -> Self {
        UserInfoSource::Unknown
    }
}

impl UserInfo {
    /// 已用流量（upload + download，任一缺失即 None）
    pub fn used(&self) -> Option<i64> {
        match (self.upload, self.download) {
            (Some(u), Some(d)) => Some(u.saturating_add(d)),
            _ => None,
        }
    }

    /// 使用百分比；total 为 0 或 None 时返回 None（不限量）
    pub fn used_percent(&self) -> Option<f32> {
        let used = self.used()?;
        let total = self.total?;
        if total <= 0 {
            return None;
        }
        Some((used as f64 / total as f64 * 100.0) as f32)
    }

    /// 剩余天数（可为负）
    pub fn days_left(&self, now: i64) -> Option<i64> {
        self.expire_at.map(|e| (e - now).div_euclid(86_400))
    }
}

/// 订阅（C.5 `Subscription`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// 8 位 base32，生成后不可变
    pub uid: String,
    pub name: String,
    /// DPAPI 密文（base64）
    pub url_cipher: String,
    /// 脱敏展示：scheme+host+path，token 段替换为 ******
    pub url_hint: String,
    pub enabled: bool,
    pub update_interval_min: u32,
    pub last_update_at: Option<i64>,
    pub last_update_status: UpdateStatus,
    pub last_error: Option<String>,
    pub mgmt: Option<UserInfo>,
    pub node_count: u32,
    pub group_names: Vec<String>,
    /// 排序用（越小越前）
    #[serde(default)]
    pub sort_order: u32,

    // ---- v3 新增：机场管理信息（纯展示，不参与拉取）----
    /// 机场登录用户名（明文：不是凭据，只是用户自己的备注性信息）
    #[serde(default)]
    pub username: Option<String>,
    /// 机场登录密码 **DPAPI 密文**（base64）；空串 = 未设置。
    /// 绝不进日志、绝不进 DTO，只在用户点「显示」时由独立命令解密。
    #[serde(default)]
    pub password_cipher: String,
    /// 机场官网地址（"打开机场"按钮用）
    #[serde(default)]
    pub homepage: Option<String>,
    /// 用户备注
    #[serde(default)]
    pub remark: Option<String>,
}

impl Subscription {
    pub fn new(
        uid: String,
        name: String,
        url_cipher: String,
        url_hint: String,
        interval_min: u32,
    ) -> Self {
        Self {
            uid,
            name,
            url_cipher,
            url_hint,
            enabled: true,
            update_interval_min: interval_min.max(15),
            last_update_at: None,
            last_update_status: UpdateStatus::Never,
            last_error: None,
            mgmt: None,
            node_count: 0,
            group_names: Vec::new(),
            sort_order: 0,
            username: None,
            password_cipher: String::new(),
            homepage: None,
            remark: None,
        }
    }

    /// 是否设置了密码（只暴露布尔，绝不暴露密文）
    pub fn has_password(&self) -> bool {
        !self.password_cipher.trim().is_empty()
    }

    /// 流量信息是否已陈旧（超过 2 个更新周期）
    pub fn userinfo_stale(&self, now: i64) -> bool {
        let Some(mgmt) = &self.mgmt else {
            return true;
        };
        let ttl = (self.update_interval_min as i64).saturating_mul(2).saturating_mul(60);
        now.saturating_sub(mgmt.fetched_at) > ttl
    }
}

/// 订阅元数据（ETag / Last-Modified，用于条件请求）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubscriptionMeta {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// 上次响应体字节数
    pub content_length: Option<u64>,
    /// 上次解析出的节点数
    pub node_count: u32,
    /// 上次拉取时间
    pub fetched_at: Option<i64>,
}

// ============================================================================
//  C.5 流量聚合（TrafficSummary）
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionTraffic {
    pub uid: String,
    pub name: String,
    pub upload: Option<i64>,
    pub download: Option<i64>,
    pub total: Option<i64>,
    pub expire_at: Option<i64>,
    pub used: Option<i64>,
    pub used_percent: Option<f32>,
    pub days_left: Option<i64>,
    pub source: UserInfoSource,
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateTraffic {
    pub upload: i64,
    pub download: i64,
    /// 全部 total 均为 0/缺失 → None（不限量）
    pub total: Option<i64>,
    /// 最早到期
    pub expire_at: Option<i64>,
    pub used: i64,
    pub used_percent: Option<f32>,
    pub subscription_count: u32,
    /// 有订阅缺少 userinfo，聚合数字仅为下限
    pub has_missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficSummary {
    pub items: Vec<SubscriptionTraffic>,
    pub aggregate: AggregateTraffic,
    pub at: i64,
}

impl TrafficSummary {
    /// 按 C.5 聚合规则构建：total 取各订阅之和（忽略 0/None）；expire_at 取最早非 None
    pub fn build(subs: &[Subscription], now: i64) -> Self {
        let mut items = Vec::with_capacity(subs.len());
        let mut sum_upload: i64 = 0;
        let mut sum_download: i64 = 0;
        let mut sum_total: i64 = 0;
        let mut any_total = false;
        let mut min_expire: Option<i64> = None;
        let mut has_missing = false;

        for s in subs.iter().filter(|s| s.enabled) {
            let mgmt = s.mgmt.clone().unwrap_or_default();
            let used = mgmt.used();
            if mgmt.upload.is_none() && mgmt.download.is_none() {
                has_missing = true;
            }
            sum_upload = sum_upload.saturating_add(mgmt.upload.unwrap_or(0));
            sum_download = sum_download.saturating_add(mgmt.download.unwrap_or(0));
            if let Some(t) = mgmt.total {
                if t > 0 {
                    sum_total = sum_total.saturating_add(t);
                    any_total = true;
                }
            }
            if let Some(e) = mgmt.expire_at {
                min_expire = Some(match min_expire {
                    Some(cur) => cur.min(e),
                    None => e,
                });
            }

            items.push(SubscriptionTraffic {
                uid: s.uid.clone(),
                name: s.name.clone(),
                upload: mgmt.upload,
                download: mgmt.download,
                total: mgmt.total,
                expire_at: mgmt.expire_at,
                used,
                used_percent: mgmt.used_percent(),
                days_left: mgmt.days_left(now),
                source: mgmt.source.clone(),
                stale: s.userinfo_stale(now),
            });
        }

        let used = sum_upload.saturating_add(sum_download);
        let total = if any_total { Some(sum_total) } else { None };
        let used_percent = match total {
            Some(t) if t > 0 => Some((used as f64 / t as f64 * 100.0) as f32),
            _ => None,
        };

        Self {
            items: items.clone(),
            aggregate: AggregateTraffic {
                upload: sum_upload,
                download: sum_download,
                total,
                expire_at: min_expire,
                used,
                used_percent,
                subscription_count: items.len() as u32,
                has_missing,
            },
            at: now,
        }
    }
}

// ============================================================================
//  C.5 测速与切换
// ============================================================================

/// 单节点探测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    /// 节点在 runtime config 中的唯一名（可能带订阅前缀）
    pub node_key: String,
    /// 原始显示名
    pub node_name: String,
    /// 延迟毫秒；None 表示失败
    pub delay_ms: Option<u32>,
    /// 失败原因（已脱敏）
    pub error: Option<String>,
    pub at: i64,
}

impl ProbeResult {
    pub fn ok(node_key: String, node_name: String, delay_ms: u32, at: i64) -> Self {
        Self {
            node_key,
            node_name,
            delay_ms: Some(delay_ms),
            error: None,
            at,
        }
    }

    pub fn fail(node_key: String, node_name: String, error: String, at: i64) -> Self {
        Self {
            node_key,
            node_name,
            delay_ms: None,
            error: Some(error),
            at,
        }
    }

    pub fn is_ok(&self) -> bool {
        self.delay_ms.is_some()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeedtestTrigger {
    /// 用户手动触发
    Manual,
    /// 定时触发
    Scheduled,
    /// 订阅更新后触发
    AfterUpdate,
    /// 自动切换失败后的补救
    Recovery,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeedtestState {
    Running,
    Done,
    Cancelled,
    Failed,
}

/// 一次测速运行
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedtestRun {
    pub run_id: String,
    pub trigger: SpeedtestTrigger,
    pub state: SpeedtestState,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub total: u32,
    pub finished: u32,
    pub ok_count: u32,
    pub results: Vec<ProbeResult>,
}

impl SpeedtestRun {
    pub fn new(run_id: String, trigger: SpeedtestTrigger, total: u32, now: i64) -> Self {
        Self {
            run_id,
            trigger,
            state: SpeedtestState::Running,
            started_at: now,
            finished_at: None,
            total,
            finished: 0,
            ok_count: 0,
            results: Vec::with_capacity(total as usize),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SwitchReason {
    Best,
    CurrentFailed,
    Manual,
    GroupMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchRecord {
    pub at: i64,
    pub group: String,
    pub from_node: Option<String>,
    pub to_node: String,
    pub from_delay: Option<u32>,
    /// Manual 切换时可能无新鲜测速数据 → None（不得用 0 冒充）
    pub to_delay: Option<u32>,
    pub reason: SwitchReason,
}

impl SwitchRecord {
    /// 最近切换历史文件上限
    pub const MAX_HISTORY: usize = 200;

    /// 追加并裁剪到上限
    pub fn push_capped(v: &mut Vec<SwitchRecord>, rec: SwitchRecord) {
        v.push(rec);
        if v.len() > Self::MAX_HISTORY {
            let drop_n = v.len() - Self::MAX_HISTORY;
            v.drain(0..drop_n);
        }
    }
}

// ============================================================================
//  C.5 运行态
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed(String),
}

impl Default for CoreState {
    fn default() -> Self {
        CoreState::Stopped
    }
}

impl CoreState {
    pub fn is_running(&self) -> bool {
        matches!(self, CoreState::Running)
    }
    pub fn is_failed(&self) -> bool {
        matches!(self, CoreState::Failed(_))
    }
}

/// 系统代理状态。`managed_by_us` 区分「我们写入的」与「用户原本的」，
/// 否则崩溃恢复会误关用户自己配的代理。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SysProxyState {
    /// 注册表当前 ProxyEnable 的实际值
    pub enabled: bool,
    /// 当前 ProxyServer 值（如 "127.0.0.1:7897"）
    pub server: Option<String>,
    /// true = 该值由本应用写入且尚未恢复
    pub managed_by_us: bool,
    /// managed_by_us 时，指向的本地混合端口
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedNode {
    pub group: String,
    pub node: String,
    pub delay_ms: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrafficCounter {
    pub up_bytes: u64,
    pub down_bytes: u64,
    pub up_rate: u64,
    pub down_rate: u64,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub core: CoreState,
    pub api_endpoint: Option<String>,
    pub active_profile_uid: Option<String>,
    pub selected: Vec<SelectedNode>,
    pub traffic: TrafficCounter,
    pub sysproxy: SysProxyState,
    pub started_at: i64,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            core: CoreState::Stopped,
            api_endpoint: None,
            active_profile_uid: None,
            selected: Vec::new(),
            traffic: TrafficCounter::default(),
            sysproxy: SysProxyState::default(),
            started_at: 0,
        }
    }
}

// ============================================================================
//  内核 API 返回类型（C.6.1 内部接口冻结面）
// ============================================================================

/// 策略组信息（来自 `GET /proxies/{name}`）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    pub name: String,
    /// mihomo API 的 `type` **原值**，如 "Selector"（大驼峰），不是 YAML 的 "select"
    pub kind: String,
    pub now: Option<String>,
    /// 组内全部成员（含 DIRECT/REJECT 等内置名）
    pub all: Vec<String>,
    pub test_url: Option<String>,
    pub hidden: bool,
}

/// 可被切换的 select 组（`proxies_selectable` 产出）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectableGroup {
    pub name: String,
    /// 组内成员。**不含** DIRECT/REJECT（用于排序）
    pub members: Vec<String>,
    pub now: Option<String>,
}

/// 内核 API 错误（C.6.1）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiError {
    /// 401 → E_CORE_API_UNAUTH
    Unauthorized,
    /// 404：组/节点不存在
    NotFound(String),
    /// 读超时
    Timeout,
    /// 其它非 2xx
    Status(u16, String),
    /// 连接失败：内核可能已退出
    Transport(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "API 鉴权失败（secret 不一致）"),
            ApiError::NotFound(w) => write!(f, "资源不存在：{w}"),
            ApiError::Timeout => write!(f, "请求超时"),
            ApiError::Status(c, b) => write!(f, "HTTP {c}: {b}"),
            ApiError::Transport(m) => write!(f, "连接失败：{m}"),
        }
    }
}

/// 手动覆盖锁定（纯内存态，不落盘、不进 RuntimeState）
#[derive(Debug, Clone)]
pub struct SwitchRuntimeLock {
    pub group: String,
    pub locked_at: i64,
}

// ============================================================================
//  用户自定义规则（v3）
// ============================================================================

/// 规则匹配类型。对应 mihomo 规则的前缀。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// 精确域名：`DOMAIN,value,policy`
    Domain,
    /// 域名后缀：`DOMAIN-SUFFIX,value,policy`
    DomainSuffix,
    /// 域名关键字：`DOMAIN-KEYWORD,value,policy`
    DomainKeyword,
    /// 目标 IP / CIDR：`IP-CIDR,value,policy`（`no-resolve` 由内核按需补）
    Ip,
    /// 进程名：`PROCESS-NAME,value,policy`
    Process,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Domain => "domain",
            RuleKind::DomainSuffix => "domain_suffix",
            RuleKind::DomainKeyword => "domain_keyword",
            RuleKind::Ip => "ip",
            RuleKind::Process => "process",
        }
    }

    /// mihomo 规则类型前缀（大写，逐字）
    pub fn mihomo_type(self) -> &'static str {
        match self {
            RuleKind::Domain => "DOMAIN",
            RuleKind::DomainSuffix => "DOMAIN-SUFFIX",
            RuleKind::DomainKeyword => "DOMAIN-KEYWORD",
            RuleKind::Ip => "IP-CIDR",
            RuleKind::Process => "PROCESS-NAME",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "domain" => RuleKind::Domain,
            "domain_suffix" => RuleKind::DomainSuffix,
            "domain_keyword" => RuleKind::DomainKeyword,
            "ip" => RuleKind::Ip,
            "process" => RuleKind::Process,
            _ => return None,
        })
    }
}

/// 规则命中后的去向
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RulePolicy {
    /// 走主策略组
    Proxy,
    /// 直连
    Direct,
    /// 拒绝
    Block,
}

impl RulePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            RulePolicy::Proxy => "proxy",
            RulePolicy::Direct => "direct",
            RulePolicy::Block => "block",
        }
    }

    /// 写入 rules 段的第三个字段
    pub fn mihomo_target(self) -> &'static str {
        match self {
            RulePolicy::Proxy => "Proxy",
            RulePolicy::Direct => "DIRECT",
            RulePolicy::Block => "REJECT",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "proxy" => RulePolicy::Proxy,
            "direct" => RulePolicy::Direct,
            "block" => RulePolicy::Block,
            _ => return None,
        })
    }
}

/// 一条用户规则
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserRule {
    /// 稳定 id（前端上下移动/删除用）
    pub id: String,
    pub kind: RuleKind,
    /// 匹配值（域名 / CIDR / 关键字 / 进程名）
    pub value: String,
    pub policy: RulePolicy,
    pub enabled: bool,
    /// 排序（越小越前；等于数组下标语义）
    #[serde(default)]
    pub sort_order: u32,
}

impl UserRule {
    /// 渲染成 mihomo rules 段的一行。
    ///
    /// `main_group` 是当前主组名——不能硬编码 "Proxy"，
    /// 万一将来主组改名，规则会指向不存在的组导致内核拒绝启动。
    pub fn to_mihomo_line(&self, main_group: &str) -> String {
        let target = match self.policy {
            RulePolicy::Proxy => main_group,
            other => other.mihomo_target(),
        };
        // IP 规则加 no-resolve：避免命中域名时多做一次 DNS 解析（延迟 + 泄漏）
        if self.kind == RuleKind::Ip {
            format!("{},{},{},no-resolve", self.kind.mihomo_type(), self.value, target)
        } else {
            format!("{},{},{}", self.kind.mihomo_type(), self.value, target)
        }
    }
}

/// 规则库根对象（落 `rules.json`）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleSet {
    #[serde(default)]
    pub rules: Vec<UserRule>,
}

impl RuleSet {
    /// 取启用中的规则，按 sort_order 升序（同序时保持插入序，稳定）
    pub fn active_sorted(&self) -> Vec<&UserRule> {
        let mut v: Vec<&UserRule> = self.rules.iter().filter(|r| r.enabled).collect();
        v.sort_by_key(|r| r.sort_order);
        v
    }

    /// 渲染成 rules 段行列表（用户规则在前，调用方再追加兜底 MATCH）
    pub fn to_mihomo_lines(&self, main_group: &str) -> Vec<String> {
        self.active_sorted()
            .into_iter()
            .map(|r| r.to_mihomo_line(main_group))
            .collect()
    }

    /// 按当前顺序重排 sort_order（0..n），保证与数组一致
    pub fn reindex(&mut self) {
        for (i, r) in self.rules.iter_mut().enumerate() {
            r.sort_order = i as u32;
        }
    }
}

// ============================================================================
//  流量统计（v3）
// ============================================================================

/// 一个统计桶（小时或天）的上下行字节
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrafficBucket {
    pub up: u64,
    pub down: u64,
}

impl TrafficBucket {
    pub fn total(&self) -> u64 {
        self.up.saturating_add(self.down)
    }

    pub fn add(&mut self, up: u64, down: u64) {
        self.up = self.up.saturating_add(up);
        self.down = self.down.saturating_add(down);
    }
}

/// 趋势图的一个点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourlyTraffic {
    /// 小时起点（Unix 秒，已对齐到整点）
    pub hour: i64,
    pub up: u64,
    pub down: u64,
}

/// 持久化的流量统计。
///
/// 只存「按天」聚合 + 「最近 N 小时」明细：
/// - 按天用于累计与今日
/// - 按小时用于趋势图（超过 24 小时的部分丢弃，避免文件无限增长）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrafficStats {
    /// 累计上行（跨天累加，永不重置）
    #[serde(default)]
    pub total_up: u64,
    /// 累计下行
    #[serde(default)]
    pub total_down: u64,
    /// 今日归属的日期（Unix 天序号；0 = 尚未初始化）
    #[serde(default)]
    pub day: i64,
    /// 今日上行
    #[serde(default)]
    pub today_up: u64,
    /// 今日下行
    #[serde(default)]
    pub today_down: u64,
    /// 最近若干小时的桶（按 hour 升序）
    #[serde(default)]
    pub hourly: Vec<HourlyTraffic>,
    /// 上次采集时刻
    #[serde(default)]
    pub updated_at: i64,
}

/// 小时桶最多保留多少个（24 小时趋势够用，多留一点防止 tick 漂移丢点）
pub const TRAFFIC_HOURLY_KEEP: usize = 48;

impl TrafficStats {
    /// Unix 秒 → 天序号（本地时区无关，按 UTC 切，够用且确定）
    pub fn day_of(ts: i64) -> i64 {
        ts.div_euclid(86_400)
    }

    /// Unix 秒 → 整点起点
    pub fn hour_of(ts: i64) -> i64 {
        ts.div_euclid(3_600) * 3_600
    }

    /// 记一次增量。`up`/`down` 是**本次采集相对上次的差值**，不是总量。
    ///
    /// 跨天时把今日清零并保留累计，保证「今日」语义正确。
    pub fn record(&mut self, now: i64, up: u64, down: u64) {
        // 1. 跨天滚动
        let today = Self::day_of(now);
        if self.day == 0 {
            self.day = today;
        } else if today != self.day {
            tracing::debug!(from = self.day, to = today, "流量统计跨天，今日计数归零");
            self.day = today;
            self.today_up = 0;
            self.today_down = 0;
        }

        // 2. 累加
        self.today_up = self.today_up.saturating_add(up);
        self.today_down = self.today_down.saturating_add(down);
        self.total_up = self.total_up.saturating_add(up);
        self.total_down = self.total_down.saturating_add(down);
        self.updated_at = now;

        // 3. 小时桶
        let hour = Self::hour_of(now);
        match self.hourly.iter_mut().find(|h| h.hour == hour) {
            Some(h) => {
                h.up = h.up.saturating_add(up);
                h.down = h.down.saturating_add(down);
            }
            None => self.hourly.push(HourlyTraffic {
                hour,
                up,
                down,
            }),
        }
        self.hourly.sort_by_key(|h| h.hour);
        if self.hourly.len() > TRAFFIC_HOURLY_KEEP {
            let drop_n = self.hourly.len() - TRAFFIC_HOURLY_KEEP;
            self.hourly.drain(0..drop_n);
        }
    }

    /// 取最近 `hours` 小时的桶（升序），缺失的小时补 0 保证曲线连续。
    pub fn recent_hours(&self, now: i64, hours: usize) -> Vec<HourlyTraffic> {
        let cur_hour = Self::hour_of(now);
        let mut out = Vec::with_capacity(hours);
        for i in (0..hours).rev() {
            let hour = cur_hour - (i as i64) * 3_600;
            match self.hourly.iter().find(|h| h.hour == hour) {
                Some(h) => out.push(HourlyTraffic {
                    hour,
                    up: h.up,
                    down: h.down,
                }),
                None => out.push(HourlyTraffic {
                    hour,
                    up: 0,
                    down: 0,
                }),
            }
        }
        out
    }
}

// ============================================================================
//  测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: i64) -> i64 {
        secs
    }

    #[test]
    fn settings_sanitize_clamps_intervals() {
        let mut s = AppSettings::default();
        s.sub_default_interval_min = 1;
        s.speedtest_interval_min = 1;
        s.speedtest_concurrency = 99;
        s.bind_address = "0.0.0.0".into();
        s.sanitize();
        assert_eq!(s.sub_default_interval_min, 15, "订阅周期下限 15 分钟");
        assert_eq!(s.speedtest_interval_min, 5, "测速周期下限 5 分钟");
        assert_eq!(s.speedtest_concurrency, 16, "并发上限 16");
        assert_eq!(s.bind_address, "127.0.0.1", "绑定地址不可改");
    }

    #[test]
    fn userinfo_derived_fields() {
        let u = UserInfo {
            upload: Some(100),
            download: Some(200),
            total: Some(1000),
            expire_at: Some(100_000),
            fetched_at: ts(0),
            source: UserInfoSource::Header,
        };
        assert_eq!(u.used(), Some(300));
        assert!((u.used_percent().unwrap() - 30.0).abs() < 0.01);
        assert_eq!(u.days_left(100_000 - 86_400), Some(1));
    }

    #[test]
    fn userinfo_unlimited_total_has_no_percent() {
        let u = UserInfo {
            upload: Some(1),
            download: Some(2),
            total: Some(0),
            expire_at: None,
            fetched_at: 0,
            source: UserInfoSource::Header,
        };
        assert_eq!(u.used(), Some(3));
        assert_eq!(u.used_percent(), None, "total=0 表示不限量");
        assert_eq!(u.days_left(0), None, "无到期日");
    }

    #[test]
    fn userinfo_missing_one_side_yields_none_used() {
        let u = UserInfo {
            upload: Some(1),
            download: None,
            total: Some(10),
            expire_at: None,
            fetched_at: 0,
            source: UserInfoSource::Unknown,
        };
        assert_eq!(u.used(), None, "任一缺失即 None");
        assert_eq!(u.used_percent(), None);
    }

    #[test]
    fn traffic_summary_aggregates_conservatively() {
        let mut a = Subscription::new("a".into(), "A".into(), "c".into(), "h".into(), 360);
        a.mgmt = Some(UserInfo {
            upload: Some(100),
            download: Some(200),
            total: Some(1000),
            expire_at: Some(200_000),
            fetched_at: 0,
            source: UserInfoSource::Header,
        });
        let mut b = Subscription::new("b".into(), "B".into(), "c".into(), "h".into(), 360);
        b.mgmt = Some(UserInfo {
            upload: Some(50),
            download: Some(50),
            total: Some(500),
            expire_at: Some(100_000),
            fetched_at: 0,
            source: UserInfoSource::Header,
        });

        let s = TrafficSummary::build(&[a, b], 0);
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.aggregate.upload, 150);
        assert_eq!(s.aggregate.download, 250);
        assert_eq!(s.aggregate.total, Some(1500));
        assert_eq!(s.aggregate.used, 400);
        assert_eq!(s.aggregate.expire_at, Some(100_000), "取最早到期");
        assert!(!s.aggregate.has_missing);
    }

    #[test]
    fn traffic_summary_ignores_disabled_and_flags_missing() {
        let mut a = Subscription::new("a".into(), "A".into(), "c".into(), "h".into(), 360);
        a.mgmt = None; // 缺 userinfo
        let mut b = Subscription::new("b".into(), "B".into(), "c".into(), "h".into(), 360);
        b.enabled = false;
        b.mgmt = Some(UserInfo {
            upload: Some(999),
            download: Some(999),
            total: Some(9999),
            expire_at: None,
            fetched_at: 0,
            source: UserInfoSource::Header,
        });

        let s = TrafficSummary::build(&[a, b], 0);
        assert_eq!(s.items.len(), 1, "停用订阅不参与");
        assert!(s.aggregate.has_missing, "缺 userinfo 需标记");
        assert_eq!(s.aggregate.total, None, "全无有效 total → 不限量");
    }

    #[test]
    fn switch_history_capped_at_200() {
        let mut v = Vec::new();
        for i in 0..250 {
            SwitchRecord::push_capped(
                &mut v,
                SwitchRecord {
                    at: i,
                    group: "g".into(),
                    from_node: None,
                    to_node: "n".into(),
                    from_delay: None,
                    to_delay: Some(1),
                    reason: SwitchReason::Best,
                },
            );
        }
        assert_eq!(v.len(), SwitchRecord::MAX_HISTORY);
        assert_eq!(v.first().unwrap().at, 50, "丢最早的");
        assert_eq!(v.last().unwrap().at, 249);
    }

    #[test]
    fn subscription_userinfo_stale_after_two_intervals() {
        let mut s = Subscription::new("a".into(), "A".into(), "c".into(), "h".into(), 60);
        s.mgmt = Some(UserInfo {
            fetched_at: 1000,
            ..Default::default()
        });
        // ttl = 60min * 2 * 60s = 7200
        assert!(!s.userinfo_stale(1000 + 7000));
        assert!(s.userinfo_stale(1000 + 7300));
    }

    #[test]
    fn switch_policy_sanitize_dedups_keywords() {
        let mut p = SwitchPolicy::default();
        p.exclude_keywords = vec!["到期".into(), " 到期 ".into(), "".into(), "流量".into()];
        p.failure_threshold = 0;
        p.sanitize();
        assert_eq!(p.exclude_keywords, vec!["到期", "流量"]);
        assert_eq!(p.failure_threshold, 1);
    }

    #[test]
    fn log_level_maps_to_mihomo_warning() {
        assert_eq!(LogLevel::Warn.to_mihomo(), "warning", "内核用 warning 不是 warn");
    }
}
