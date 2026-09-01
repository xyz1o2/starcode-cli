/// 路径工具
///
/// 对标claude-code-main的src/utils/path.ts
use std::path::{Path, PathBuf};

/// 规范化路径
pub fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let parts: Vec<&str> = path.split('/').collect();
    let mut result = Vec::new();

    for part in parts {
        match part {
            "." => {}
            ".." => {
                result.pop();
            }
            "" => {
                if result.is_empty() {
                    result.push("");
                }
            }
            _ => {
                result.push(part);
            }
        }
    }

    if result.is_empty() {
        ".".to_string()
    } else {
        result.join("/")
    }
}

/// 连接路径
pub fn join(base: &str, relative: &str) -> String {
    let base = normalize(base);
    let relative = normalize(relative);

    if relative.starts_with('/') {
        relative
    } else if base.ends_with('/') {
        format!("{}{}", base, relative)
    } else {
        format!("{}/{}", base, relative)
    }
}

/// 获取文件名
pub fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// 获取目录名
pub fn dirname(path: &str) -> &str {
    Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".")
}

/// 获取文件扩展名
pub fn extension(path: &str) -> Option<&str> {
    Path::new(path).extension().and_then(|e| e.to_str())
}

/// 获取不带扩展名的文件名
pub fn stem(path: &str) -> &str {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
}

/// 检查路径是否为绝对路径
pub fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
}

/// 检查路径是否为相对路径
pub fn is_relative(path: &str) -> bool {
    !is_absolute(path)
}

/// 转换为绝对路径
pub fn to_absolute(path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

/// 计算相对路径
pub fn relative_to(base: &str, target: &str) -> String {
    let base = Path::new(base);
    let target = Path::new(target);

    match pathdiff::diff_paths(target, base) {
        Some(rel) => rel.to_string_lossy().to_string(),
        None => target.to_string_lossy().to_string(),
    }
}

/// 检查路径是否在目录下
pub fn is_under(path: &str, dir: &str) -> bool {
    let path = normalize(path);
    let dir = normalize(dir);

    path.starts_with(&dir)
}

/// 获取文件MIME类型
pub fn mime_type(path: &str) -> &'static str {
    match extension(path) {
        Some("json") => "application/json",
        Some("xml") => "text/xml",
        Some("html") | Some("htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("ts") => "application/typescript",
        Some("md") => "text/markdown",
        Some("txt") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}
