//! Windows 系统代理开关（WinINET 注册表 + `InternetSetOption` 刷新）。
//!
//! 设计要点（08 章）：
//! 1. 写注册表后**必须**调 `InternetSetOption(INTERNET_OPTION_SETTINGS_CHANGED /
//!    REFRESH)`，否则已打开的应用（Chrome、微信）不认新设置，用户会以为开关没生效。
//! 2. 开启前先写 **journal 快照**，记录用户原本的 `ProxyEnable`/`ProxyServer`。
//!    崩溃后下次启动据此恢复，避免把用户的代理永久改成本机端口。
//! 3. `managed_by_us` 语义：只恢复我们自己改的，用户自己配的绝不动。

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use windows_sys::Win32::Networking::WinInet::{
    InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
};

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::domain::model::SysProxyState;
use crate::system::paths::{read_json_recover, write_json, Paths};

/// 注册表路径：`HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`
const REG_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

/// 本机代理始终绕过这些地址，避免回环请求被自己劫持
const BYPASS_DEFAULT: &str = "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*;<local>";

/// journal 快照（落盘，用于崩溃恢复）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SysProxyJournal {
    /// 开启前用户的原始 ProxyEnable
    pub previous_enable: bool,
    /// 开启前用户的原始 ProxyServer
    pub previous_server: Option<String>,
    /// 开启前用户的原始 ProxyOverride
    pub previous_override: Option<String>,
    /// 是否处于「我们已设置」状态
    pub active: bool,
    pub at: i64,
}

pub struct SysProxyManager {
    journal_path: std::path::PathBuf,
    /// 串行化注册表操作，避免与托盘点击/自动恢复竞争
    lock: Mutex<()>,
    current: Mutex<SysProxyState>,
}

impl SysProxyManager {
    pub fn new(paths: &Paths) -> Self {
        Self {
            journal_path: paths.sysproxy_journal(),
            lock: Mutex::new(()),
            current: Mutex::new(SysProxyState::default()),
        }
    }

    /// 启动时调用：若上次是异常退出且代理仍是我们设的，恢复用户原值。
    ///
    /// 返回是否执行了恢复。
    pub fn recover_if_needed(&self) -> AppResult<bool> {
        let journal: Option<SysProxyJournal> = read_json_recover(&self.journal_path)?;
        let Some(j) = journal else {
            return Ok(false);
        };
        if !j.active {
            return Ok(false);
        }

        let cur_enabled = read_proxy_enable().unwrap_or(false);
        let cur_server = read_proxy_server().unwrap_or_default();

        // 只有当前值仍是我们写入的形态（指向回环），才恢复
        let looks_like_ours = cur_server.contains("127.0.0.1") || cur_server.is_empty();
        if !looks_like_ours {
            tracing::warn!(
                server = %redact(&cur_server),
                "检测到用户已自行修改系统代理，跳过恢复以免覆盖用户设置"
            );
            self.clear_journal();
            return Ok(false);
        }

        tracing::warn!(
            enabled = cur_enabled,
            prev_enable = j.previous_enable,
            "检测到上次未正常退出（系统代理未还原），正在恢复"
        );

        write_proxy(j.previous_enable, j.previous_server.as_deref(), j.previous_override.as_deref())?;
        refresh_wininet();
        self.clear_journal();

        if let Ok(mut c) = self.current.lock() {
            *c = SysProxyState {
                enabled: j.previous_enable,
                server: j.previous_server.clone(),
                managed_by_us: false,
                port: None,
            };
        }
        Ok(true)
    }

    /// 当前状态（读注册表真实值，不读缓存的幻想值）
    pub fn read_state(&self) -> SysProxyState {
        let enabled = read_proxy_enable().unwrap_or(false);
        let server = read_proxy_server().ok().filter(|s| !s.trim().is_empty());
        let managed = self
            .cache()
            .map(|c| c.managed_by_us)
            .unwrap_or(false);
        let port = self.cache().and_then(|c| c.port);
        SysProxyState {
            enabled,
            server,
            managed_by_us: managed,
            port,
        }
    }

    fn cache(&self) -> Option<SysProxyState> {
        self.current.lock().ok().map(|g| g.clone())
    }

    /// 开启系统代理，指向本机 `127.0.0.1:<port>`
    pub fn enable(&self, port: u16) -> AppResult<SysProxyState> {
        let _g = self.lock.lock().map_err(|_| AppError::msg(ErrorCode::Internal, "代理锁中毒"))?;

        let prev_enable = read_proxy_enable().unwrap_or(false);
        let prev_server = read_proxy_server().ok().filter(|s| !s.trim().is_empty());
        let prev_override = read_proxy_override().ok();

        // 已是我们设的同一端口 → 幂等返回，不重复写 journal
        let already_ours = prev_enable
            && prev_server
                .as_deref()
                .map(|s| s.contains(&format!("127.0.0.1:{port}")))
                .unwrap_or(false);

        if !already_ours {
            // 先落 journal 再改注册表：顺序反了就没有恢复依据
            let journal = SysProxyJournal {
                previous_enable: prev_enable,
                previous_server: prev_server.clone(),
                previous_override: prev_override.clone(),
                active: true,
                at: crate::util::time::now_secs(),
            };
            write_json(&self.journal_path, &journal)?;
        }

        let server = format!("127.0.0.1:{port}");
        write_proxy(true, Some(&server), Some(BYPASS_DEFAULT))?;
        refresh_wininet();

        // 回读校验：注册表写了不等于生效，必须确认
        let verify_enabled = read_proxy_enable().unwrap_or(false);
        let verify_server = read_proxy_server().unwrap_or_default();
        if !verify_enabled || !verify_server.contains(&format!("127.0.0.1:{port}")) {
            // 写失败 → 立刻回滚，不能留下半开状态
            let _ = write_proxy(prev_enable, prev_server.as_deref(), prev_override.as_deref());
            refresh_wininet();
            return Err(AppError::msg(
                ErrorCode::SysproxyFailed,
                "系统代理写入后回读校验失败，已回滚",
            ));
        }

        let state = SysProxyState {
            enabled: true,
            server: Some(server),
            managed_by_us: true,
            port: Some(port),
        };
        if let Ok(mut c) = self.current.lock() {
            *c = state.clone();
        }
        tracing::info!(port, "系统代理已开启");
        Ok(state)
    }

    /// 关闭系统代理并恢复用户原值
    pub fn disable(&self) -> AppResult<SysProxyState> {
        let _g = self.lock.lock().map_err(|_| AppError::msg(ErrorCode::Internal, "代理锁中毒"))?;

        let journal: Option<SysProxyJournal> = read_json_recover(&self.journal_path)?;
        let (enable, server, override_) = match &journal {
            Some(j) if j.active => (
                j.previous_enable,
                j.previous_server.clone(),
                j.previous_override.clone(),
            ),
            // 没有 journal：说明不是我们开的，只关闭不猜用户原值
            _ => (false, None, None),
        };

        write_proxy(enable, server.as_deref(), override_.as_deref())?;
        refresh_wininet();
        self.clear_journal();

        let state = SysProxyState {
            enabled: enable,
            server: server.clone(),
            managed_by_us: false,
            port: None,
        };
        if let Ok(mut c) = self.current.lock() {
            *c = state.clone();
        }
        tracing::info!(restored_enable = enable, "系统代理已关闭并恢复原设置");
        Ok(state)
    }

    fn clear_journal(&self) {
        let _ = std::fs::remove_file(&self.journal_path);
    }
}

fn redact(s: &str) -> String {
    crate::util::redact::sanitize(s)
}

// ============================================================================
//  注册表读写
// ============================================================================

fn open_settings_key(write: bool) -> AppResult<winreg::RegKey> {
    use winreg::enums::KEY_READ;
    use winreg::RegKey;
    // 注意：必须用 winreg 自己 re-export 的 HKEY_CURRENT_USER（isize），
    // 不能用 windows_sys 里同名常量（*mut c_void），二者 ABI 表示不同。
    let hkcu = RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let mut o = hkcu.open_subkey_with_flags(REG_PATH, KEY_READ);
    if write {
        use winreg::enums::KEY_WRITE;
        o = hkcu.open_subkey_with_flags(REG_PATH, KEY_READ | KEY_WRITE);
    }
    o.map_err(|e| AppError::with_detail(ErrorCode::SysproxyFailed, e))
}

pub fn read_proxy_enable() -> AppResult<bool> {
    let key = open_settings_key(false)?;
    // ProxyEnable 是 REG_DWORD；不存在时视为关闭
    let v: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    Ok(v != 0)
}

pub fn read_proxy_server() -> AppResult<String> {
    let key = open_settings_key(false)?;
    Ok(key.get_value::<String, _>("ProxyServer").unwrap_or_default())
}

fn read_proxy_override() -> AppResult<String> {
    let key = open_settings_key(false)?;
    Ok(key.get_value::<String, _>("ProxyOverride").unwrap_or_default())
}

/// 写三项。`server` 为 None 时删除该值（恢复"未设置"的原始状态）
fn write_proxy(enable: bool, server: Option<&str>, override_: Option<&str>) -> AppResult<()> {
    let key = open_settings_key(true)?;

    key.set_value("ProxyEnable", &(if enable { 1u32 } else { 0u32 }))
        .map_err(|e| AppError::with_detail(ErrorCode::SysproxyFailed, e))?;

    match server.filter(|s| !s.trim().is_empty()) {
        Some(s) => {
            key.set_value("ProxyServer", &s.to_string())
                .map_err(|e| AppError::with_detail(ErrorCode::SysproxyFailed, e))?;
        }
        None => {
            let _ = key.delete_value("ProxyServer");
        }
    }

    match override_.filter(|s| !s.trim().is_empty()) {
        Some(o) => {
            let _ = key.set_value("ProxyOverride", &o.to_string());
        }
        None => {
            // 关闭时若用户原本没有 ProxyOverride，删掉我们写的
            let _ = key.delete_value("ProxyOverride");
        }
    }
    Ok(())
}

/// 通知系统设置已变更。
///
/// 少了这一步，已运行的程序（浏览器、微信）不会立即使用新代理，
/// 用户会以为软件没生效。
pub fn refresh_wininet() {
    // SAFETY: 这两个调用只传 null 句柄 + 选项常量，系统不要求额外生命周期。
    unsafe {
        InternetSetOptionW(std::ptr::null_mut(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null_mut(), 0);
        InternetSetOptionW(std::ptr::null_mut(), INTERNET_OPTION_REFRESH, std::ptr::null_mut(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bypass_list_covers_loopback_and_lan() {
        assert!(BYPASS_DEFAULT.contains("127.*"));
        assert!(BYPASS_DEFAULT.contains("localhost"));
        assert!(BYPASS_DEFAULT.contains("192.168.*"));
        assert!(BYPASS_DEFAULT.contains("<local>"));
    }

    #[test]
    fn journal_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        let j = SysProxyJournal {
            previous_enable: false,
            previous_server: Some("user-proxy:8080".into()),
            previous_override: Some("*.corp.com".into()),
            active: true,
            at: 123,
        };
        write_json(&paths.sysproxy_journal(), &j).unwrap();
        let back: Option<SysProxyJournal> = read_json_recover(&paths.sysproxy_journal()).unwrap();
        let back = back.unwrap();
        assert_eq!(back.previous_server.as_deref(), Some("user-proxy:8080"));
        assert!(back.active);
    }

    #[test]
    fn recover_noop_without_journal() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();
        let mgr = SysProxyManager::new(&paths);
        // 无 journal 时不应改动注册表
        assert!(!mgr.recover_if_needed().unwrap());
    }

    #[test]
    fn recover_noop_when_journal_inactive() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.runtime_dir()).unwrap();

        let j = SysProxyJournal {
            active: false,
            ..Default::default()
        };
        write_json(&paths.sysproxy_journal(), &j).unwrap();

        let mgr = SysProxyManager::new(&paths);
        assert!(!mgr.recover_if_needed().unwrap(), "active=false 不该恢复");
    }
}
