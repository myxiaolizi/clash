//! 运行时路径解析（契约 C.3）。
//!
//! - Roaming: `%APPDATA%\ClashZhengl\`  → 设置、订阅索引与原始配置
//! - Local  : `%LOCALAPPDATA%\ClashZhengl\` → 内核、日志、运行时配置

use std::path::{Path, PathBuf};

use once_cell::sync::OnceCell;

use crate::domain::errors::{AppError, AppResult, ErrorCode};

pub const APP_DIR_NAME: &str = "ClashZhengl";

static ROOT: OnceCell<Paths> = OnceCell::new();

/// 全部路径一次性解析，避免各处重复拼接出不一致的目录
#[derive(Debug, Clone)]
pub struct Paths {
    /// Roaming 根
    pub roaming: PathBuf,
    /// Local 根
    pub local: PathBuf,
}

impl Paths {
    pub fn resolve() -> AppResult<Self> {
        // 数据根目录覆盖：便携模式与端到端测试用。
        //
        // 为什么需要它：Windows 上 `dirs::config_dir()` 走的是
        // SHGetKnownFolderPath（已知文件夹 API），**不读 APPDATA 环境变量** ——
        // 只改环境变量做隔离是无效的，daemon 照样读写真实用户数据。
        // 所以必须提供一条显式的覆盖通道。
        //
        // 用法：设 `CZ_DATA_ROOT=<dir>`，roaming 与 local 都落在其下。
        if let Some(root) = std::env::var_os("CZ_DATA_ROOT") {
            let root = PathBuf::from(root);
            if !root.as_os_str().is_empty() {
                return Ok(Self {
                    roaming: root.join("roaming"),
                    local: root.join("local"),
                });
            }
        }
        let roaming = dirs::config_dir()
            .ok_or_else(|| AppError::new(ErrorCode::StoreIo))?
            .join(APP_DIR_NAME);
        let local = dirs::data_local_dir()
            .ok_or_else(|| AppError::new(ErrorCode::StoreIo))?
            .join(APP_DIR_NAME);
        Ok(Self { roaming, local })
    }

    /// 覆盖根目录（测试用）
    pub fn with_roots(roaming: PathBuf, local: PathBuf) -> Self {
        Self { roaming, local }
    }

    /// 全局单例，首次调用时创建目录
    pub fn global() -> AppResult<&'static Self> {
        if let Some(p) = ROOT.get() {
            return Ok(p);
        }
        let p = Self::resolve()?;
        p.ensure_dirs()?;
        let _ = ROOT.set(p);
        ROOT.get()
            .ok_or_else(|| AppError::new(ErrorCode::StoreIo))
    }

    /// 创建全部运行所需目录（幂等）
    pub fn ensure_all(&self) -> AppResult<()> {
        self.ensure_dirs()
    }

    /// 测试用：显式注入（仅首次生效）
    pub fn init_with(roaming: PathBuf, local: PathBuf) -> AppResult<&'static Self> {
        let p = Self::with_roots(roaming, local);
        p.ensure_dirs()?;
        let _ = ROOT.set(p);
        ROOT.get().ok_or_else(|| AppError::new(ErrorCode::StoreIo))
    }

    fn ensure_dirs(&self) -> AppResult<()> {
        for d in [
            self.roaming.clone(),
            self.local.clone(),
            self.profiles_dir(),
            self.runtime_dir(),
            self.core_root(),
            self.download_dir(),
            self.logs_dir(),
            self.diagnostics_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| {
                AppError::with_detail(ErrorCode::StoreIo, format!("创建目录失败 {}: {e}", d.display()))
            })?;
        }
        Ok(())
    }

    // ---------- Roaming ----------

    pub fn settings_file(&self) -> PathBuf {
        self.roaming.join("settings.json")
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.roaming.join("profiles")
    }

    pub fn profiles_index(&self) -> PathBuf {
        self.profiles_dir().join("index.json")
    }

    /// 订阅原始 YAML
    pub fn profile_yaml(&self, uid: &str) -> PathBuf {
        self.profiles_dir().join(format!("{uid}.yaml"))
    }

    /// 订阅元数据（ETag 等）
    pub fn profile_meta(&self, uid: &str) -> PathBuf {
        self.profiles_dir().join(format!("{uid}.meta.json"))
    }

    // ---------- Local ----------

    pub fn runtime_dir(&self) -> PathBuf {
        self.local.join("runtime")
    }

    /// 合并后交给内核的配置
    pub fn runtime_config(&self) -> PathBuf {
        self.runtime_dir().join("config.yaml")
    }

    /// external-controller 的 secret
    pub fn secret_file(&self) -> PathBuf {
        self.runtime_dir().join("secret")
    }

    /// 内核可执行文件根（按版本分子目录）
    pub fn core_root(&self) -> PathBuf {
        self.runtime_dir().join("core")
    }

    /// 指定版本的内核路径
    pub fn core_binary(&self, version: &str) -> PathBuf {
        self.core_root().join(version).join("mihomo.exe")
    }

    pub fn core_dir(&self, version: &str) -> PathBuf {
        self.core_root().join(version)
    }

    /// 内核下载临时区
    pub fn download_dir(&self) -> PathBuf {
        self.runtime_dir().join("download")
    }

    /// 系统代理快照（崩溃恢复守卫）
    pub fn sysproxy_journal(&self) -> PathBuf {
        self.runtime_dir().join("sysproxy.journal.json")
    }

    /// 最近切换历史
    pub fn switch_history(&self) -> PathBuf {
        self.runtime_dir().join("switch-history.json")
    }

    /// 用户自定义规则库（Roaming：属于用户数据，跟着设置一起备份）
    pub fn rules_file(&self) -> PathBuf {
        self.roaming.join("rules.json")
    }

    /// 流量统计（Roaming：同样是用户数据）
    pub fn traffic_stats_file(&self) -> PathBuf {
        self.roaming.join("traffic-stats.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.local.join("logs")
    }

    pub fn diagnostics_dir(&self) -> PathBuf {
        self.local.join("diagnostics")
    }

    /// 内核工作目录（`-d` 参数）
    pub fn core_workdir(&self) -> PathBuf {
        self.runtime_dir()
    }

    /// 内核持久化缓存（geodata、cache.db 都落这里，避免污染 config 目录）
    pub fn core_cache_dir(&self) -> PathBuf {
        self.runtime_dir().join("core-cache")
    }
}

/// 原子写：写 `.tmp` → fsync → rename（C.3 强制要求）
pub fn atomic_write(path: &Path, data: &[u8]) -> AppResult<()> {
    use std::io::Write;

    let dir = path
        .parent()
        .ok_or_else(|| AppError::msg(ErrorCode::StoreIo, "目标路径没有父目录"))?;
    std::fs::create_dir_all(dir)?;

    let tmp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.flush()?;
        // 关键：必须 fsync，否则断电会留下 0 字节文件
        f.sync_all()?;
    }

    // Windows 上 rename 到已存在文件需要先删除目标（ReplaceFile 语义由 OS 保证原子性）
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(AppError::with_detail(
                ErrorCode::StoreIo,
                format!("重命名失败 {}: {e}", path.display()),
            ))
        }
    }
}

/// 读取 JSON；损坏时重命名为 `*.corrupt-<ts>` 并返回 None（不得崩溃）
pub fn read_json_recover<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> AppResult<Option<T>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(AppError::with_detail(ErrorCode::StoreIo, e)),
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => Ok(Some(v)),
        Err(e) => {
            // 备份坏文件后返回 None，让调用方用默认值继续
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let bak = path.with_extension(format!("corrupt-{ts}"));
            let _ = std::fs::rename(path, &bak);
            tracing::warn!(
                target: "store",
                path = %path.display(),
                backup = %bak.display(),
                err = %e,
                "JSON 损坏，已备份并回退默认值"
            );
            Ok(None)
        }
    }
}

pub fn write_json<T: serde::Serialize>(path: &Path, v: &T) -> AppResult<()> {
    let data = serde_json::to_vec_pretty(v)?;
    atomic_write(path, &data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量是进程级全局状态，涉及它的测试必须串行，否则并行跑会互相覆盖
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn atomic_write_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("test.json");
        let val = serde_json::json!({"a": 1, "中文": "值"});
        write_json(&p, &val).unwrap();
        let back: Option<serde_json::Value> = read_json_recover(&p).unwrap();
        assert_eq!(back.unwrap(), val);
        // 临时文件不应残留
        let leftovers: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "原子写不得残留 tmp 文件");
    }

    #[test]
    fn corrupt_json_is_backed_up_not_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.json");
        std::fs::write(&p, b"{not json").unwrap();
        let r: Option<serde_json::Value> = read_json_recover(&p).unwrap();
        assert!(r.is_none(), "损坏时返回 None 而非报错");
        assert!(!p.exists(), "原文件应被移走");
        let has_backup = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("corrupt-"));
        assert!(has_backup, "应留下 .corrupt-<ts> 备份");
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope.json");
        let r: Option<serde_json::Value> = read_json_recover(&p).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn data_root_override_is_used_for_both_roots() {
        // 回归：Windows 上 dirs 走已知文件夹 API，不认 APPDATA 环境变量，
        // 因此端到端测试必须靠 CZ_DATA_ROOT 才能真隔离。
        // 这里直接验证 resolve() 的分支逻辑（用临时值，测完复原）。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("czroot");

        // 串行执行：环境变量是进程全局的，并行测试会互相干扰
        let _guard = ENV_LOCK.lock();
        let old = std::env::var_os("CZ_DATA_ROOT");
        std::env::set_var("CZ_DATA_ROOT", &root);

        let p = Paths::resolve().unwrap();
        assert_eq!(p.roaming, root.join("roaming"));
        assert_eq!(p.local, root.join("local"));

        match old {
            Some(v) => std::env::set_var("CZ_DATA_ROOT", v),
            None => std::env::remove_var("CZ_DATA_ROOT"),
        }
    }

    #[test]
    fn empty_data_root_falls_back_to_system_dirs() {
        let _guard = ENV_LOCK.lock();
        let old = std::env::var_os("CZ_DATA_ROOT");
        std::env::set_var("CZ_DATA_ROOT", "");

        let p = Paths::resolve().unwrap();
        // 空串必须被忽略，回落到真实系统目录（以 ClashZhengl 结尾）
        assert!(
            p.roaming.ends_with(APP_DIR_NAME),
            "空覆盖值不应生效，实际 {}",
            p.roaming.display()
        );

        match old {
            Some(v) => std::env::set_var("CZ_DATA_ROOT", v),
            None => std::env::remove_var("CZ_DATA_ROOT"),
        }
    }

    #[test]
    fn paths_layout_matches_contract() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::with_roots(dir.path().join("roaming"), dir.path().join("local"));
        assert!(p.settings_file().ends_with("settings.json"));
        assert!(p
            .profiles_index()
            .ends_with(Path::new("profiles").join("index.json")));
        assert!(p.profile_yaml("abc123").ends_with("abc123.yaml"));
        assert!(p.runtime_config().ends_with(Path::new("runtime").join("config.yaml")));
        assert!(p.sysproxy_journal().ends_with("sysproxy.journal.json"));
        assert!(p.switch_history().ends_with("switch-history.json"));
        assert!(p.rules_file().ends_with("rules.json"));
        assert!(p.traffic_stats_file().ends_with("traffic-stats.json"));
        assert!(p.core_binary("v1.19.33").ends_with(Path::new("v1.19.33").join("mihomo.exe")));
    }
}
