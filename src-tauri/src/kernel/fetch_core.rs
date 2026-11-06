//! 内核下载、校验、解压、版本管理（P1）。
//!
//! 资产命名规则见附录 A.7：
//! `mihomo-windows-amd64-compatible-v<ver>.zip` 优先（GOAMD64=v1，老 CPU 兼容性最好）。

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::errors::{AppError, AppResult, ErrorCode};
use crate::system::paths::{atomic_write, Paths};

/// 默认内核版本
pub const DEFAULT_CORE_VERSION: &str = "v1.19.33";

/// 下载源前缀（可换成镜像）
const GITHUB_RELEASE_BASE: &str = "https://github.com/MetaCubeX/mihomo/releases/download";

/// 已安装内核信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledCore {
    pub version: String,
    pub path: PathBuf,
    pub size: u64,
    pub installed_at: i64,
}

/// 按架构选择资产名。
///
/// 注意：不能硬编码 x64（C.1 要求代码中不假设架构）。
pub fn asset_name(version: &str, arch: &str) -> AppResult<String> {
    let a = match arch {
        "x86_64" => "amd64-compatible",
        "aarch64" => "arm64",
        other => {
            return Err(AppError::msg(
                ErrorCode::CoreDownloadFailed,
                format!("不支持的系统架构：{other}"),
            ))
        }
    };
    Ok(format!("mihomo-windows-{a}-{version}.zip"))
}

pub fn download_url(version: &str, arch: &str) -> AppResult<String> {
    let asset = asset_name(version, arch)?;
    Ok(format!("{GITHUB_RELEASE_BASE}/{version}/{asset}"))
}

/// 下载并安装内核。
///
/// 返回安装后的可执行文件路径。
pub async fn download_and_install(
    paths: &Paths,
    version: &str,
    proxy: Option<&str>,
    progress: impl Fn(u64, Option<u64>) + Send + Sync,
) -> AppResult<InstalledCore> {
    let arch = std::env::consts::ARCH;
    let url = download_url(version, arch)?;

    let target_dir = paths.core_dir(version);
    let exe_path = target_dir.join("mihomo.exe");

    // 已存在且能跑 → 直接复用
    if exe_path.exists() {
        if let Ok(v) = probe_core_version(&exe_path).await {
            tracing::info!(version = %v, "内核已存在，跳过下载");
            return Ok(InstalledCore {
                version: version.to_string(),
                path: exe_path.clone(),
                size: std::fs::metadata(&exe_path).map(|m| m.len()).unwrap_or(0),
                installed_at: crate::util::time::now_secs(),
            });
        }
    }

    std::fs::create_dir_all(&target_dir)?;
    std::fs::create_dir_all(paths.download_dir())?;

    let part_path = paths.download_dir().join(format!("{version}.zip.part"));
    let zip_path = paths.download_dir().join(format!("{version}.zip"));

    // ---- 复用已下载完成的 zip ----
    // 场景：上次下载 + 解压之间失败（如磁盘满、进程被杀），
    // 重试时不该重新拉 20MB。
    if zip_path.exists() && !part_path.exists() {
        let size = std::fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);
        if size > 1024 * 1024 {
            tracing::info!(size, "复用已下载的内核压缩包，跳过网络下载");
            extract_mihomo(&zip_path, &target_dir)?;
            let actual = probe_core_version(&exe_path).await?;
            tracing::info!(actual = %actual, "复用压缩包安装内核完成");
            return Ok(InstalledCore {
                version: version.to_string(),
                path: exe_path,
                size: std::fs::metadata(target_dir.join("mihomo.exe"))
                    .map(|m| m.len())
                    .unwrap_or(0),
                installed_at: crate::util::time::now_secs(),
            });
        }
    }

    tracing::info!(url = %url, "开始下载内核");
    let client = crate::util::net::build_download_client(proxy)?;

    let resp = client
        .get(&url)
        // 连接超时：避免 DNS/握手阶段无限等待
        .timeout(std::time::Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::CoreDownloadFailed, e.without_url()))?;

    if !resp.status().is_success() {
        return Err(AppError::msg(
            ErrorCode::CoreDownloadFailed,
            format!("HTTP {}，可能是版本号不存在或网络受限", resp.status()),
        ));
    }

    let total = resp.content_length();
    let mut downloaded: u64 = 0;
    let mut file = std::fs::File::create(&part_path)?;
    {
        use std::io::Write;
        let mut stream = resp;
        while let Some(chunk) = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            stream.chunk(),
        )
        .await
        .map_err(|_| {
            // 单块数据 60 秒没到 → 连接实际上已死，中断而不是无限挂起
            AppError::msg(ErrorCode::CoreDownloadFailed, "下载中断：60 秒未收到数据")
        })?
        .map_err(|e| AppError::with_detail(ErrorCode::CoreDownloadFailed, e.without_url()))?
        {
            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            progress(downloaded, total);
        }
        file.flush()?;
        file.sync_all()?;
    }
    drop(file);

    if let Some(t) = total {
        if downloaded != t {
            let _ = std::fs::remove_file(&part_path);
            return Err(AppError::msg(
                ErrorCode::CoreDownloadFailed,
                format!("下载不完整：{downloaded}/{t} 字节"),
            ));
        }
    }

    std::fs::rename(&part_path, &zip_path)?;
    tracing::info!(downloaded, "内核压缩包下载完成");

    // 解压（zip 保留：若后续 -v 校验失败，重试可复用，不必重下 20MB）
    extract_mihomo(&zip_path, &target_dir)?;

    if !exe_path.exists() {
        return Err(AppError::msg(
            ErrorCode::CoreDownloadFailed,
            "解压后未生成 mihomo.exe",
        ));
    }

    // 校验：能跑起来并报版本
    let actual = probe_core_version(&exe_path).await?;
    tracing::info!(expected = %version, actual = %actual, "内核安装完成并校验通过");

    // 校验通过才删 zip
    let _ = std::fs::remove_file(&zip_path);

    let size = std::fs::metadata(&exe_path).map(|m| m.len()).unwrap_or(0);
    Ok(InstalledCore {
        version: version.to_string(),
        path: exe_path,
        size,
        installed_at: crate::util::time::now_secs(),
    })
}

/// 从 zip 中提取内核可执行文件，统一重命名为 `mihomo.exe`。
///
/// **不能硬匹配 `mihomo.exe`**：mihomo 官方 release 的可执行文件名带平台后缀，
/// 例如 `mihomo-windows-amd64-compatible-v1.19.33.exe`。不同版本/变体的文件名还会变
/// （`-compatible`、`-go120`、`-amd64v3` 等），因此改为：
/// 1. 优先精确匹配 `mihomo.exe`（旧版/自编译包）
/// 2. 否则取任意以 `.exe` 结尾、且名字里含 `mihomo` 的条目
/// 3. 再退一步取任何 `.exe`（排除目录）
///
/// 大小校验：内核实际 50MB+，小于 1MB 基本是占位文件或错误页，直接报错。
fn extract_mihomo(zip_path: &Path, out_dir: &Path) -> AppResult<()> {
    const MIN_CORE_BYTES: u64 = 1024 * 1024; // 1 MB

    let f = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(f)
        .map_err(|e| AppError::with_detail(ErrorCode::CoreDownloadFailed, e))?;

    // ---- 先扫一遍挑出候选（记录 index，避免二次借用冲突）----
    let mut best: Option<(usize, String, u64, u8)> = None; // (idx, name, size, rank)
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .map_err(|e| AppError::with_detail(ErrorCode::CoreDownloadFailed, e))?;
        let name = entry.name().to_string();
        let lower = name.to_ascii_lowercase();

        // 跳过目录
        if entry.is_dir() || lower.ends_with('/') {
            continue;
        }
        if !lower.ends_with(".exe") {
            continue;
        }

        // rank 越小越优先
        let rank = if lower.ends_with("/mihomo.exe") || lower == "mihomo.exe" {
            0
        } else if lower.contains("mihomo") {
            1
        } else {
            2
        };

        let size = entry.size();
        // 同等优先级时取最大的（多架构包里 compatible 版通常最大且最通用）
        let better = match &best {
            None => true,
            Some((_, _, cur_size, cur_rank)) => rank < *cur_rank || (rank == *cur_rank && size > *cur_size),
        };
        if better {
            best = Some((i, name, size, rank));
        }
    }

    let Some((idx, name, size, _rank)) = best else {
        return Err(AppError::msg(
            ErrorCode::CoreDownloadFailed,
            "压缩包内没有 .exe（可能下载到了错误页或架构不匹配）",
        ));
    };

    if size < MIN_CORE_BYTES {
        return Err(AppError::msg(
            ErrorCode::CoreDownloadFailed,
            format!("压缩包内的 {name} 仅 {size} 字节，疑似损坏"),
        ));
    }

    // ---- 解压选中的条目 ----
    let mut entry = archive
        .by_index(idx)
        .map_err(|e| AppError::with_detail(ErrorCode::CoreDownloadFailed, e))?;
    let mut buf = Vec::with_capacity(size as usize);
    entry.read_to_end(&mut buf)?;
    drop(entry);

    let dest = out_dir.join("mihomo.exe");
    atomic_write(&dest, &buf)?;
    tracing::info!(entry = %name, bytes = buf.len(), dest = %dest.display(), "内核已提取");
    Ok(())
}

/// 运行 `mihomo -v` 验证可执行文件有效，返回版本串
pub async fn probe_core_version(exe: &Path) -> AppResult<String> {
    if !exe.exists() {
        return Err(AppError::new(ErrorCode::CoreNotFound));
    }
    let out = tokio::process::Command::new(exe)
        .arg("-v")
        .output()
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::CoreNotFound, e))?;

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let text = format!("{stdout}{stderr}");

    // mihomo -v 输出形如 "Mihomo Meta v1.19.33 windows amd64 with go1.23..."
    let ver = text
        .split_whitespace()
        .find(|t| t.starts_with('v') && t.chars().nth(1).is_some_and(|c| c.is_ascii_digit()))
        .map(|s| s.to_string());

    match ver {
        Some(v) => Ok(v),
        None => Err(AppError::msg(
            ErrorCode::CoreVerifyFailed,
            format!("无法识别内核版本，输出：{}", crate::domain::errors::truncate(&text, 200)),
        )),
    }
}

/// 列出已安装的内核版本
pub fn list_installed(paths: &Paths) -> Vec<InstalledCore> {
    let root = paths.core_root();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let exe = e.path().join("mihomo.exe");
        if !exe.exists() {
            continue;
        }
        let version = e.file_name().to_string_lossy().to_string();
        let size = std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0);
        let installed_at = std::fs::metadata(&exe)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(InstalledCore {
            version,
            path: exe,
            size,
            installed_at,
        });
    }
    out.sort_by(|a, b| b.version.cmp(&a.version));
    out
}

/// 计算文件 SHA256（供诊断与完整性核对）
pub fn sha256_file(path: &Path) -> AppResult<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_name_uses_compatible_build_for_x64() {
        let n = asset_name("v1.19.33", "x86_64").unwrap();
        assert_eq!(n, "mihomo-windows-amd64-compatible-v1.19.33.zip");
        assert!(n.contains("compatible"), "x64 必须选 compatible 变体，兼容老 CPU");
    }

    #[test]
    fn asset_name_for_arm64() {
        let n = asset_name("v1.19.33", "aarch64").unwrap();
        assert_eq!(n, "mihomo-windows-arm64-v1.19.33.zip");
    }

    #[test]
    fn asset_name_rejects_unknown_arch() {
        let e = asset_name("v1.19.33", "mips").unwrap_err();
        assert_eq!(e.code, ErrorCode::CoreDownloadFailed);
    }

    #[test]
    fn download_url_is_wellformed() {
        let u = download_url("v1.19.33", "x86_64").unwrap();
        assert!(u.starts_with("https://github.com/MetaCubeX/mihomo/releases/download/v1.19.33/"));
        assert!(u.ends_with(".zip"));
    }

    #[test]
    fn sha256_of_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f.txt");
        std::fs::write(&p, b"abc").unwrap();
        // "abc" 的 SHA256 是公认值
        assert_eq!(
            sha256_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn list_installed_empty_when_no_cores() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        std::fs::create_dir_all(paths.core_root()).unwrap();
        assert!(list_installed(&paths).is_empty());
    }

    #[test]
    fn list_installed_reads_version_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_roots(dir.path().join("r"), dir.path().join("l"));
        for v in ["v1.18.0", "v1.19.33"] {
            let d = paths.core_dir(v);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("mihomo.exe"), b"fake").unwrap();
        }
        // 无 exe 的目录应被忽略
        std::fs::create_dir_all(paths.core_dir("v0.0.1")).unwrap();

        let list = list_installed(&paths);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version, "v1.19.33", "新的在前");
    }

    // ------------------------------------------------------------------
    //  extract_mihomo：真实 release 包里的文件名并不叫 mihomo.exe
    // ------------------------------------------------------------------

    /// 合成一个 zip。`files` 为 (内部路径, 内容)。
    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        use std::io::Write as _;
        let f = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in files {
            zw.start_file(*name, opts).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    #[test]
    fn extract_accepts_official_platform_suffixed_name() {
        // 这就是 v1.19.33 官方包里真实的文件名，早期实现硬匹配 mihomo.exe 会失败
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("core.zip");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let payload = vec![0x4du8; 2 * 1024 * 1024]; // 2MB 占位，过最小体积校验
        make_zip(
            &zip,
            &[("mihomo-windows-amd64-compatible-v1.19.33.exe", &payload)],
        );

        extract_mihomo(&zip, &out).unwrap();
        let exe = out.join("mihomo.exe");
        assert!(exe.exists(), "必须重命名为 mihomo.exe");
        assert_eq!(std::fs::metadata(&exe).unwrap().len(), payload.len() as u64);
    }

    #[test]
    fn extract_prefers_exact_mihomo_exe() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("core.zip");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let big = vec![1u8; 2 * 1024 * 1024];
        let small = vec![2u8; 1024 * 1024];
        make_zip(
            &zip,
            &[
                ("mihomo.exe", &small),
                ("mihomo-windows-amd64-v1.19.33.exe", &big),
            ],
        );

        extract_mihomo(&zip, &out).unwrap();
        let got = std::fs::read(out.join("mihomo.exe")).unwrap();
        assert_eq!(got.len(), small.len(), "应优先精确匹配 mihomo.exe");
    }

    #[test]
    fn extract_rejects_zip_without_exe() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("core.zip");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        make_zip(&zip, &[("readme.txt", b"hello"), ("config.yaml", b"x: 1")]);
        let err = extract_mihomo(&zip, &out).unwrap_err();
        assert_eq!(err.code, ErrorCode::CoreDownloadFailed);
        assert!(err.message.contains(".exe"));
    }

    #[test]
    fn extract_rejects_tiny_exe() {
        // 下载到错误页/占位文件时不能当成内核装上
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("core.zip");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        make_zip(&zip, &[("mihomo.exe", b"<html>404</html>")]);
        let err = extract_mihomo(&zip, &out).unwrap_err();
        assert_eq!(err.code, ErrorCode::CoreDownloadFailed);
        assert!(err.message.contains("损坏") || err.message.contains("字节"));
    }

    #[test]
    fn extract_skips_directories() {
        let dir = tempfile::tempdir().unwrap();
        let zip = dir.path().join("core.zip");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();

        let payload = vec![9u8; 2 * 1024 * 1024];
        make_zip(
            &zip,
            &[("subdir/", b""), ("subdir/mihomo-windows-amd64.exe", &payload)],
        );
        extract_mihomo(&zip, &out).unwrap();
        assert!(out.join("mihomo.exe").exists(), "子目录中的内核也要能提取");
    }
}
