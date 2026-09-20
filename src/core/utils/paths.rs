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

/// 词法规范化路径，消除 `.` 和可安全折叠的 `..`，但不解析符号链接。
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            std::path::Component::Prefix(..) | std::path::Component::RootDir => {
                normalized.push(component);
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if normalized
                    .components()
                    .next_back()
                    .is_some_and(|component| matches!(component, std::path::Component::Normal(..)))
                {
                    normalized.pop();
                } else if !normalized.is_absolute() {
                    normalized.push(component);
                }
            }
            std::path::Component::Normal(component) => normalized.push(component),
        }
    }

    normalized
}

/// `GlobalState::read_file_state` 的键。
///
/// 所有读写这张表的工具（Read / Edit / Write / multi_edit / read_many /
/// Glob / 搜索）必须用同一个函数算键。曾经各写各的：Read/Edit 走
/// `canonicalize`，而 read_many / Glob / 搜索直接把原始路径字符串塞进去。
/// 于是「Glob 浏览过 → Edit」这类流程在表里查不到记录，Edit 就报
/// 「must be read with `Read` before using `Edit`」——明明读过，却拦下来
/// 要求再读一遍。这种漏判是偶发的（只在绕过 Read 的路径下出现），所以
/// 长期没被发现。
///
/// 优先 `canonicalize`：它解符号链接、折叠 `..`、在 Windows 上统一大小写，
/// 是唯一能让「同一文件的不同写法」落到同一键上的手段。文件不存在或文件系统
/// 不支持时退回 `normalize_path`（纯词法）——此时所有调用方拿到的仍是同一份
/// 退化结果，键依然一致。
pub fn read_state_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| normalize_path(path))
        .to_string_lossy()
        .to_string()
}

pub fn resolve_tool_path(base_dir: &Path, raw_path: &str) -> PathBuf {
    let normalized = normalize_cross_platform_path(raw_path);
    let resolved = if normalized.is_absolute() {
        normalized
    } else {
        base_dir.join(normalized)
    };
    normalize_path(&resolved)
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
        let cache = PROJECT_FILE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
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
                let mut cache = PROJECT_FILE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
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

    let mut cache = PROJECT_FILE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_path_does_not_escape_an_absolute_root() {
        assert_eq!(
            normalize_path(Path::new("/../../workspace/file.rs")),
            PathBuf::from("/workspace/file.rs")
        );
    }

    #[test]
    fn resolve_tool_path_normalizes_relative_parent_components() {
        assert_eq!(
            resolve_tool_path(Path::new("/workspace/project"), "nested/../src/file.rs"),
            PathBuf::from("/workspace/project/src/file.rs")
        );
    }

    #[test]
    fn read_state_key_maps_path_variants_to_one_key() {
        // 同一文件的不同写法必须落到同一个键。Glob / 搜索 / read_many 写表时
        // 拿到的可能是带 `..` 或相对基准目录的路径，Edit 查表时用的是规范化后的
        // 路径；两者不一致就会明明读过却报 [edit_file_not_read]。
        let direct = read_state_key(Path::new("/workspace/project/src/lib.rs"));
        let with_dotdot =
            read_state_key(Path::new("/workspace/project/src/nested/../sub/../lib.rs"));
        assert_eq!(direct, with_dotdot);
    }

    #[test]
    fn read_state_key_matches_after_resolve_tool_path() {
        // 工具侧两步走：先 resolve_tool_path 补全成绝对路径，再 read_state_key
        // 规范化。真实文件上这两步的结果要和直接 canonicalize 一致，否则
        // 「Glob 浏览过 → Edit」的流程在表里仍然查不到记录。
        let dir = std::env::temp_dir().join("starcode_read_state_key_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lib.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();

        let via_tool = read_state_key(&resolve_tool_path(&dir, "./nested/../lib.rs"));
        let canonical = file.canonicalize().unwrap();
        assert_eq!(via_tool, canonical.to_string_lossy().to_string());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
