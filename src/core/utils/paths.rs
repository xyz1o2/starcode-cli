use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const STAR_DIR: &str = ".star";
pub const GOOGLE_ACCOUNTS_FILENAME: &str = "google_accounts.json";

pub fn tildeify_path(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(home_str) = home.to_str() {
            if path.starts_with(home_str) {
                return path.replacen(home_str, "~", 1);
            }
        }
    }
    path.to_string()
}

pub fn shorten_path(file_path: &str, max_len: usize) -> String {
    if file_path.len() <= max_len {
        return file_path.to_string();
    }

    // Try to preserve filename
    let path = Path::new(file_path);
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.len() >= max_len {
            // Filename itself is too long, just truncate end
            return format!("...{}", &file_path[file_path.len() - (max_len - 3)..]);
        }

        let remaining = max_len - name.len() - 4; // 3 for "..." + 1 for separator
        if remaining > 0 {
            return format!(
                "...{}{}{}",
                std::path::MAIN_SEPARATOR,
                &file_path[file_path.len() - remaining..file_path.len() - name.len()],
                name
            );
        }
    }

    format!("...{}", &file_path[file_path.len() - (max_len - 3)..])
}

pub fn make_relative(path: &Path, base: &Path) -> PathBuf {
    path.strip_prefix(base).unwrap_or(path).to_path_buf()
}

pub fn escape_path(path: &str) -> String {
    path.replace(" ", "\\ ")
}

pub fn unescape_path(path: &str) -> String {
    path.replace("\\ ", " ")
}

pub fn get_project_hash(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn is_subpath(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

pub fn current_dir_cached() -> &'static PathBuf {
    static CWD: OnceLock<PathBuf> = OnceLock::new();
    CWD.get_or_init(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn normalize_cross_platform_path(raw_path: &str) -> PathBuf {
    normalize_cross_platform_path_for_env(raw_path, detected_path_environment())
}

pub fn resolve_tool_path(base_dir: &Path, raw_path: &str) -> PathBuf {
    let normalized = normalize_cross_platform_path(raw_path);
    if normalized.is_absolute() {
        normalized
    } else {
        base_dir.join(normalized)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathEnvironment {
    Windows,
    Wsl,
    Unix,
}

fn detected_path_environment() -> PathEnvironment {
    if cfg!(windows) {
        return PathEnvironment::Windows;
    }

    if is_running_under_wsl() {
        return PathEnvironment::Wsl;
    }

    PathEnvironment::Unix
}

fn is_running_under_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/version")
            .map(|value| value.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false)
}

fn normalize_cross_platform_path_for_env(raw_path: &str, env: PathEnvironment) -> PathBuf {
    match env {
        PathEnvironment::Windows => {
            if let Some(windows_path) = malformed_wsl_path_to_windows_drive_path(raw_path) {
                return windows_path;
            }
            PathBuf::from(raw_path)
        }
        PathEnvironment::Wsl => {
            if let Some(wsl_path) = malformed_windows_prefixed_wsl_path(raw_path) {
                return wsl_path;
            }
            if let Some(wsl_path) = windows_drive_path_to_wsl(raw_path) {
                return wsl_path;
            }
            PathBuf::from(raw_path)
        }
        PathEnvironment::Unix => {
            // 处理 Windows UNC 路径格式 (\\?\...)
            if let Some(unc_path) = windows_unc_path_to_unix(raw_path) {
                return unc_path;
            }
            PathBuf::from(raw_path)
        }
    }
}

/// 将 Windows UNC 路径 (\\?\...) 转换为 Unix 路径
/// 例如: \\?\H:\test\yolo_train_web -> /mnt/h/test/yolo_train_web
fn windows_unc_path_to_unix(raw_path: &str) -> Option<PathBuf> {
    // 检查是否是 UNC 路径格式
    if !raw_path.starts_with("\\\\?\\") && !raw_path.starts_with("//?/") {
        return None;
    }

    // 提取驱动器字母和路径
    let path_part = &raw_path[4..]; // 跳过 \\?\
    let normalized = path_part.replace('\\', "/");

    // 检查是否是驱动器路径 (如 H:\...)
    let bytes = normalized.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = &normalized[3..];
        return Some(PathBuf::from(format!("/mnt/{}/{}", drive, rest)));
    }

    None
}

pub fn malformed_windows_prefixed_wsl_path(raw_path: &str) -> Option<PathBuf> {
    let normalized = raw_path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() < 8
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'/'
        || !normalized[3..].starts_with("mnt/")
    {
        return None;
    }

    Some(PathBuf::from(format!("/{}", &normalized[3..])))
}

fn malformed_wsl_path_to_windows_drive_path(raw_path: &str) -> Option<PathBuf> {
    let normalized = raw_path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() < 10
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || bytes[2] != b'/'
        || !normalized[3..].starts_with("mnt/")
    {
        return None;
    }

    let drive = (bytes[0] as char).to_ascii_uppercase();
    let mounted_drive = normalized.as_bytes().get(7).copied()? as char;
    if mounted_drive.to_ascii_uppercase() != drive {
        return None;
    }

    let rest = normalized.get(8..).unwrap_or("").trim_start_matches('/');
    Some(PathBuf::from(format!("{}:/{}", drive, rest)))
}

pub fn windows_drive_path_to_wsl(raw_path: &str) -> Option<PathBuf> {
    let bytes = raw_path.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'\\' | b'/')
    {
        return None;
    }

    let drive = (bytes[0] as char).to_ascii_lowercase();
    let rest = raw_path[3..].replace('\\', "/");
    Some(PathBuf::from(format!("/mnt/{}/{}", drive, rest)))
}

/// 智能路径解析
///
/// 支持：
/// - 相对路径
/// - 绝对路径
/// - ~ 家目录
/// - 环境变量
pub fn resolve_path(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();

    // 1. 处理空路径
    if path.is_empty() {
        return Err("Path is empty".to_string());
    }

    // 2. 展开 ~ (家目录)
    let path = if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            if path == "~" {
                home
            } else {
                home.join(&path[2..])
            }
        } else {
            return Err("Cannot determine home directory".to_string());
        }
    } else {
        normalize_cross_platform_path(path)
    };

    // 3. 规范化路径
    match path.canonicalize() {
        Ok(canonical) => Ok(canonical),
        Err(_) => {
            // 如果文件不存在，至少返回绝对路径
            if path.is_absolute() {
                Ok(path)
            } else {
                Ok(current_dir_cached().join(&path))
            }
        }
    }
}

pub fn get_mcp_config_path() -> PathBuf {
    current_project_star_dir().join("mcp.json")
}

pub fn project_star_dir(project_root: &Path) -> PathBuf {
    project_root.join(STAR_DIR)
}

/// 缓存 find_project_file_upwards 的结果，避免启动时重复遍历目录树
/// 在 WSL2/NFS/网络文件系统上，每次 exists() 调用都可能涉及昂贵的内核往返
static PROJECT_FILE_CACHE: Lazy<Mutex<HashMap<(PathBuf, Vec<String>), Option<PathBuf>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub fn find_project_file_upwards(start: &Path, candidates: &[&str]) -> Option<PathBuf> {
    // 检查缓存
    let cache_key = {
        let owned_candidates: Vec<String> = candidates.iter().map(|&s| s.to_string()).collect();
        (start.to_path_buf(), owned_candidates)
    };
    {
        let cache = PROJECT_FILE_CACHE.lock().unwrap();
        if let Some(result) = cache.get(&cache_key) {
            return result.clone();
        }
    }

    let home = dirs::home_dir();
    let git_root = start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf);

    for dir in start.ancestors() {
        for candidate in candidates {
            let path = dir.join(candidate);
            if path.exists() {
                let result = Some(path);
                let mut cache = PROJECT_FILE_CACHE.lock().unwrap();
                cache.insert(cache_key, result.clone());
                return result;
            }
        }

        if git_root.as_deref() == Some(dir) {
            break;
        }

        if git_root.is_none() && home.as_deref() == Some(dir) && dir != start {
            break;
        }
    }

    let mut cache = PROJECT_FILE_CACHE.lock().unwrap();
    cache.insert(cache_key, None);
    None
}

pub fn find_nearest_existing_star_dir(start: &Path) -> Option<PathBuf> {
    find_project_file_upwards(start, &[STAR_DIR])
}

pub fn current_project_star_dir() -> PathBuf {
    let cwd = current_dir_cached().clone();
    find_nearest_existing_star_dir(&cwd).unwrap_or_else(|| cwd.join(STAR_DIR))
}
