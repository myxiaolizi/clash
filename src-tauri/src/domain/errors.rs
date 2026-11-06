//! 统一错误类型与错误码（契约 C.8）。
//!
//! 铁律：**错误码只从此处定义**，任何模块不得自造字符串错误码。
//! 前端据 `code` 做分支，据 `message` 展示，据 `retryable` 决定是否显示"重试"。

use serde::{Deserialize, Serialize};

/// 错误码枚举（C.8）。序列化为 `E_*` 字面量，与契约逐字一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    // ---- 通用 ----
    #[serde(rename = "E_INTERNAL")]
    Internal,
    #[serde(rename = "E_INVALID_ARG")]
    InvalidArg,
    #[serde(rename = "E_BUSY")]
    Busy,
    #[serde(rename = "E_NOT_FOUND")]
    NotFound,
    #[serde(rename = "E_CANCELLED")]
    Cancelled,

    // ---- 网络 ----
    #[serde(rename = "E_NET_TIMEOUT")]
    NetTimeout,
    #[serde(rename = "E_NET_DNS")]
    NetDns,
    #[serde(rename = "E_NET_TLS")]
    NetTls,
    #[serde(rename = "E_NET_PROXY")]
    NetProxy,
    #[serde(rename = "E_HTTP_STATUS")]
    HttpStatus,
    #[serde(rename = "E_NET_OFFLINE")]
    NetOffline,

    // ---- 订阅 / 解析 ----
    #[serde(rename = "E_SUB_URL_INVALID")]
    SubUrlInvalid,
    #[serde(rename = "E_SUB_EMPTY")]
    SubEmpty,
    #[serde(rename = "E_PARSE_YAML")]
    ParseYaml,
    #[serde(rename = "E_PARSE_EMPTY")]
    ParseEmpty,
    #[serde(rename = "E_USERINFO_MISSING")]
    UserinfoMissing,
    #[serde(rename = "E_NO_SUBSCRIPTION")]
    NoSubscription,

    // ---- 存储 ----
    #[serde(rename = "E_STORE_IO")]
    StoreIo,
    #[serde(rename = "E_STORE_CORRUPT")]
    StoreCorrupt,
    #[serde(rename = "E_STORE_CRYPTO")]
    StoreCrypto,

    // ---- 内核 ----
    #[serde(rename = "E_CORE_NOT_FOUND")]
    CoreNotFound,
    #[serde(rename = "E_CORE_START_FAILED")]
    CoreStartFailed,
    #[serde(rename = "E_CORE_API_UNAUTH")]
    CoreApiUnauth,
    #[serde(rename = "E_CORE_PORT_EXHAUSTED")]
    CorePortExhausted,
    #[serde(rename = "E_CORE_DOWNLOAD_FAILED")]
    CoreDownloadFailed,
    #[serde(rename = "E_CORE_VERIFY_FAILED")]
    CoreVerifyFailed,
    #[serde(rename = "E_CORE_NOT_RUNNING")]
    CoreNotRunning,

    // ---- 配置 ----
    #[serde(rename = "E_CONFIG_INVALID")]
    ConfigInvalid,
    #[serde(rename = "E_CONFIG_WRITE")]
    ConfigWrite,

    // ---- 权限 / 系统 ----
    #[serde(rename = "E_PERM_ADMIN")]
    PermAdmin,
    #[serde(rename = "E_SYSPROXY_FAILED")]
    SysproxyFailed,
    #[serde(rename = "E_AUTOSTART_FAILED")]
    AutostartFailed,

    // ---- 测速 / 切换 ----
    #[serde(rename = "E_SPEEDTEST_RUNNING")]
    SpeedtestRunning,
    #[serde(rename = "E_SPEEDTEST_NO_NODE")]
    SpeedtestNoNode,
    #[serde(rename = "E_SWITCH_GROUP_MISSING")]
    SwitchGroupMissing,
    #[serde(rename = "E_SWITCH_NODE_MISSING")]
    SwitchNodeMissing,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Internal => "E_INTERNAL",
            ErrorCode::InvalidArg => "E_INVALID_ARG",
            ErrorCode::Busy => "E_BUSY",
            ErrorCode::NotFound => "E_NOT_FOUND",
            ErrorCode::Cancelled => "E_CANCELLED",
            ErrorCode::NetTimeout => "E_NET_TIMEOUT",
            ErrorCode::NetDns => "E_NET_DNS",
            ErrorCode::NetTls => "E_NET_TLS",
            ErrorCode::NetProxy => "E_NET_PROXY",
            ErrorCode::HttpStatus => "E_HTTP_STATUS",
            ErrorCode::NetOffline => "E_NET_OFFLINE",
            ErrorCode::SubUrlInvalid => "E_SUB_URL_INVALID",
            ErrorCode::SubEmpty => "E_SUB_EMPTY",
            ErrorCode::ParseYaml => "E_PARSE_YAML",
            ErrorCode::ParseEmpty => "E_PARSE_EMPTY",
            ErrorCode::UserinfoMissing => "E_USERINFO_MISSING",
            ErrorCode::NoSubscription => "E_NO_SUBSCRIPTION",
            ErrorCode::StoreIo => "E_STORE_IO",
            ErrorCode::StoreCorrupt => "E_STORE_CORRUPT",
            ErrorCode::StoreCrypto => "E_STORE_CRYPTO",
            ErrorCode::CoreNotFound => "E_CORE_NOT_FOUND",
            ErrorCode::CoreStartFailed => "E_CORE_START_FAILED",
            ErrorCode::CoreApiUnauth => "E_CORE_API_UNAUTH",
            ErrorCode::CorePortExhausted => "E_CORE_PORT_EXHAUSTED",
            ErrorCode::CoreDownloadFailed => "E_CORE_DOWNLOAD_FAILED",
            ErrorCode::CoreVerifyFailed => "E_CORE_VERIFY_FAILED",
            ErrorCode::CoreNotRunning => "E_CORE_NOT_RUNNING",
            ErrorCode::ConfigInvalid => "E_CONFIG_INVALID",
            ErrorCode::ConfigWrite => "E_CONFIG_WRITE",
            ErrorCode::PermAdmin => "E_PERM_ADMIN",
            ErrorCode::SysproxyFailed => "E_SYSPROXY_FAILED",
            ErrorCode::AutostartFailed => "E_AUTOSTART_FAILED",
            ErrorCode::SpeedtestRunning => "E_SPEEDTEST_RUNNING",
            ErrorCode::SpeedtestNoNode => "E_SPEEDTEST_NO_NODE",
            ErrorCode::SwitchGroupMissing => "E_SWITCH_GROUP_MISSING",
            ErrorCode::SwitchNodeMissing => "E_SWITCH_NODE_MISSING",
        }
    }

    /// 该错误是否值得让用户点"重试"
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::NetTimeout
                | ErrorCode::NetDns
                | ErrorCode::NetTls
                | ErrorCode::NetProxy
                | ErrorCode::HttpStatus
                | ErrorCode::NetOffline
                | ErrorCode::StoreIo
                | ErrorCode::CoreStartFailed
                | ErrorCode::CoreDownloadFailed
                | ErrorCode::CorePortExhausted
                | ErrorCode::SysproxyFailed
        )
    }

    /// 默认中文提示（可被具体错误覆盖）
    pub fn default_message(self) -> &'static str {
        match self {
            ErrorCode::Internal => "内部错误，请查看日志",
            ErrorCode::InvalidArg => "参数不合法",
            ErrorCode::Busy => "操作正在进行中，请稍候",
            ErrorCode::NotFound => "对象不存在",
            ErrorCode::Cancelled => "操作已取消",
            ErrorCode::NetTimeout => "网络请求超时",
            ErrorCode::NetDns => "域名解析失败",
            ErrorCode::NetTls => "TLS 握手失败",
            ErrorCode::NetProxy => "代理连接失败",
            ErrorCode::HttpStatus => "服务器返回错误状态码",
            ErrorCode::NetOffline => "当前无网络连接",
            ErrorCode::SubUrlInvalid => "订阅地址格式不正确",
            ErrorCode::SubEmpty => "订阅内容为空",
            ErrorCode::ParseYaml => "订阅配置解析失败",
            ErrorCode::ParseEmpty => "订阅中没有可用节点",
            ErrorCode::UserinfoMissing => "该订阅未提供流量信息",
            ErrorCode::NoSubscription => "还没有添加任何订阅",
            ErrorCode::StoreIo => "本地文件读写失败",
            ErrorCode::StoreCorrupt => "本地配置已损坏，已重置",
            ErrorCode::StoreCrypto => "凭据解密失败",
            ErrorCode::CoreNotFound => "未找到内核程序",
            ErrorCode::CoreStartFailed => "内核启动失败",
            ErrorCode::CoreApiUnauth => "内核鉴权失败",
            ErrorCode::CorePortExhausted => "找不到可用端口",
            ErrorCode::CoreDownloadFailed => "内核下载失败",
            ErrorCode::CoreVerifyFailed => "内核文件校验失败",
            ErrorCode::CoreNotRunning => "内核未运行",
            ErrorCode::ConfigInvalid => "配置校验未通过",
            ErrorCode::ConfigWrite => "配置文件写入失败",
            ErrorCode::PermAdmin => "该操作需要管理员权限",
            ErrorCode::SysproxyFailed => "系统代理设置失败",
            ErrorCode::AutostartFailed => "开机自启设置失败",
            ErrorCode::SpeedtestRunning => "测速正在进行中",
            ErrorCode::SpeedtestNoNode => "没有可测速的节点",
            ErrorCode::SwitchGroupMissing => "目标策略组不存在",
            ErrorCode::SwitchNodeMissing => "目标节点不存在",
        }
    }
}

/// 应用错误。跨 IPC 边界时序列化为 `{code, message, detail, retryable}`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    /// 排障细节（已脱敏、已截断），可空
    pub detail: Option<String>,
    pub retryable: bool,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.default_message().to_string(),
            detail: None,
            retryable: code.retryable(),
        }
    }

    pub fn with_detail(code: ErrorCode, detail: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: code.default_message().to_string(),
            detail: Some(truncate(&detail.to_string(), 300)),
            retryable: code.retryable(),
        }
    }

    pub fn msg(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            retryable: code.retryable(),
        }
    }

    pub fn internal(detail: impl std::fmt::Display) -> Self {
        Self::with_detail(ErrorCode::Internal, detail)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)?;
        if let Some(d) = &self.detail {
            write!(f, " ({d})")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

/// 截断字符串（按字符边界，不破 UTF-8）
pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

// ---- 便捷构造 ----

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::with_detail(ErrorCode::StoreIo, e)
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::with_detail(ErrorCode::StoreCorrupt, e)
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        let code = if e.is_timeout() {
            ErrorCode::NetTimeout
        } else if e.is_connect() {
            ErrorCode::NetOffline
        } else if e.is_decode() {
            ErrorCode::ParseYaml
        } else {
            ErrorCode::NetProxy
        };
        AppError::with_detail(code, e.without_url())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::with_detail(ErrorCode::Internal, e)
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ---- Tauri IPC 序列化：错误对象直接作为 Err 的载荷 ----
impl serde::Serialize for AppErrorPayload {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

/// 包装层，让 `AppResult` 能直接作为 Tauri command 返回类型
pub struct AppErrorPayload(pub AppError);

impl From<AppError> for AppErrorPayload {
    fn from(e: AppError) -> Self {
        AppErrorPayload(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_strings_match_contract() {
        // 抽查 C.8 中的关键码，防止改名后前端分支失效
        assert_eq!(ErrorCode::Internal.as_str(), "E_INTERNAL");
        assert_eq!(ErrorCode::NetTimeout.as_str(), "E_NET_TIMEOUT");
        assert_eq!(ErrorCode::HttpStatus.as_str(), "E_HTTP_STATUS");
        assert_eq!(ErrorCode::ParseEmpty.as_str(), "E_PARSE_EMPTY");
        assert_eq!(ErrorCode::CoreApiUnauth.as_str(), "E_CORE_API_UNAUTH");
        assert_eq!(ErrorCode::SwitchGroupMissing.as_str(), "E_SWITCH_GROUP_MISSING");
        assert_eq!(ErrorCode::SpeedtestNoNode.as_str(), "E_SPEEDTEST_NO_NODE");
    }

    #[test]
    fn error_code_serializes_to_literal() {
        let j = serde_json::to_string(&ErrorCode::NetTimeout).unwrap();
        assert_eq!(j, "\"E_NET_TIMEOUT\"", "serde rename 必须与契约一致");
        let j2 = serde_json::to_string(&ErrorCode::PermAdmin).unwrap();
        assert_eq!(j2, "\"E_PERM_ADMIN\"");
    }

    #[test]
    fn truncate_respects_utf8_boundary() {
        let s = "中文测试".repeat(100);
        let t = truncate(&s, 10);
        assert_eq!(t.chars().count(), 11); // 10 + ellipsis
        assert!(t.ends_with('…'));
    }

    #[test]
    fn retryable_flags() {
        assert!(ErrorCode::NetTimeout.retryable());
        assert!(ErrorCode::CoreStartFailed.retryable());
        assert!(!ErrorCode::InvalidArg.retryable());
        assert!(!ErrorCode::ParseEmpty.retryable());
    }
}
