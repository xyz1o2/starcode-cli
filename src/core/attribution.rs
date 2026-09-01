//! Attribution Header 模块
//!
//! 对标 Claude Code 的 Attribution Header：
//! - 包含版本号、构建时间、入口点标识
//! - 用于计费和安全验证
//! - 始终不缓存（因版本和指纹不同而变化）

use std::sync::OnceLock;

// ── 版本信息 ──

/// CLI 版本号（从 Cargo.toml 读取）
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 构建时间（编译时确定）
const BUILD_TIME: &str = "2026-08-08";

/// 入口点标识
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entrypoint {
    /// 交互式 REPL
    Repl,
    /// Headless 模式（单次 prompt）
    Headless,
    /// SDK 调用
    Sdk,
    /// 管道模式
    Pipe,
    /// 子命令
    Subcommand(String),
}

impl std::fmt::Display for Entrypoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Entrypoint::Repl => write!(f, "repl"),
            Entrypoint::Headless => write!(f, "headless"),
            Entrypoint::Sdk => write!(f, "sdk"),
            Entrypoint::Pipe => write!(f, "pipe"),
            Entrypoint::Subcommand(name) => write!(f, "subcommand:{}", name),
        }
    }
}

// ── Attribution Header ──

/// Attribution Header 内容
#[derive(Debug, Clone)]
pub struct AttributionHeader {
    /// CLI 版本
    pub version: String,
    /// 构建时间
    pub build_time: String,
    /// 入口点
    pub entrypoint: Entrypoint,
    /// 客户端指纹（用于安全验证）
    pub fingerprint: String,
}

impl AttributionHeader {
    /// 创建新的 Attribution Header
    pub fn new(entrypoint: Entrypoint) -> Self {
        Self {
            version: VERSION.to_string(),
            build_time: BUILD_TIME.to_string(),
            entrypoint,
            fingerprint: generate_fingerprint(),
        }
    }

    /// 格式化为字符串（用于 System Prompt 注入）
    pub fn format_header(&self) -> String {
        format!(
            "[StarCode CLI v{} | {} | {}]",
            self.version, self.entrypoint, self.build_time
        )
    }

    /// 格式化为详细字符串（用于调试）
    pub fn format_detailed(&self) -> String {
        format!(
            "StarCode CLI v{}\nBuild: {}\nEntrypoint: {}\nFingerprint: {}",
            self.version, self.build_time, self.entrypoint, self.fingerprint
        )
    }

    /// 转换为 JSON 值（用于 API 请求头）
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "cc_version": format!("{}+{}", self.version, self.fingerprint),
            "cc_entrypoint": self.entrypoint.to_string(),
            "cc_build_time": self.build_time,
        })
    }

    /// 获取版本号
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 获取构建时间
    pub fn build_time(&self) -> &str {
        &self.build_time
    }

    /// 获取入口点
    pub fn entrypoint(&self) -> &Entrypoint {
        &self.entrypoint
    }

    /// 获取指纹
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

// ── 指纹生成 ──

/// 生成客户端指纹
///
/// 指纹用于验证请求来自真实的 StarCode CLI 客户端。
/// 基于版本号、构建时间和随机数生成。
fn generate_fingerprint() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    VERSION.hash(&mut hasher);
    BUILD_TIME.hash(&mut hasher);

    // 添加一些运行时信息
    if let Ok(hostname) = hostname::get() {
        hostname.to_string_lossy().hash(&mut hasher);
    }

    let hash = hasher.finish();
    format!("{:016x}", hash)
}

/// 获取 hostname（如果可用）
mod hostname {
    use std::ffi::OsString;

    pub fn get() -> Result<OsString, std::io::Error> {
        #[cfg(unix)]
        {
            Ok(std::env::var("HOSTNAME")
                .map(OsString::from)
                .unwrap_or_else(|_| OsString::from("unknown")))
        }

        #[cfg(windows)]
        {
            Ok(std::env::var("COMPUTERNAME")
                .map(OsString::from)
                .unwrap_or_else(|_| OsString::from("unknown")))
        }
    }
}

// ── 全局实例 ──

/// 全局 Attribution Header 实例
fn global_attribution_header() -> &'static AttributionHeader {
    static HEADER: OnceLock<AttributionHeader> = OnceLock::new();
    HEADER.get_or_init(|| AttributionHeader::new(Entrypoint::Repl))
}

/// 获取全局 Attribution Header
pub fn get_attribution_header() -> &'static AttributionHeader {
    global_attribution_header()
}

/// 初始化 Attribution Header（在程序启动时调用）
pub fn init_attribution_header(entrypoint: Entrypoint) -> &'static AttributionHeader {
    static HEADER: OnceLock<AttributionHeader> = OnceLock::new();
    HEADER.get_or_init(|| AttributionHeader::new(entrypoint))
}

// ── 辅助函数 ──

/// 获取 CLI 版本
pub fn get_version() -> &'static str {
    VERSION
}

/// 获取构建时间
pub fn get_build_time() -> &'static str {
    BUILD_TIME
}

/// 检查是否为开发版本
pub fn is_dev_version() -> bool {
    VERSION.contains("dev") || VERSION.contains("alpha") || VERSION.contains("beta")
}

/// 获取用户代理字符串
pub fn get_user_agent() -> String {
    format!("StarCode-CLI/{}", VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribution_header_format() {
        let header = AttributionHeader::new(Entrypoint::Repl);
        let formatted = header.format_header();
        assert!(formatted.contains("StarCode CLI"));
        assert!(formatted.contains(VERSION));
    }

    #[test]
    fn test_attribution_header_json() {
        let header = AttributionHeader::new(Entrypoint::Headless);
        let json = header.to_json();
        assert!(json.get("cc_version").is_some());
        assert!(json.get("cc_entrypoint").is_some());
        assert_eq!(json["cc_entrypoint"], "headless");
    }

    #[test]
    fn test_version_info() {
        assert!(!get_version().is_empty());
        assert!(!get_build_time().is_empty());
    }
}
