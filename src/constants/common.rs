/// 通用常量

/// 版本信息
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");
pub const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

/// 产品信息
pub const PRODUCT_NAME: &str = "StarCode CLI";
pub const PRODUCT_DESCRIPTION: &str = "AI-powered coding assistant";
pub const PRODUCT_URL: &str = "https://github.com/starcode/starcode-cli";

/// 默认配置
pub const DEFAULT_MODEL: &str = "gpt-4o";
pub const DEFAULT_PROVIDER: &str = "openai";
pub const DEFAULT_LANGUAGE: &str = "en";
pub const DEFAULT_THEME: &str = "dark";

/// 时间格式
pub const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub const DATE_FORMAT: &str = "%Y-%m-%d";
pub const TIME_FORMAT: &str = "%H:%M:%S";

/// 文件大小限制
pub const MAX_FILE_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10MB
pub const MAX_FILE_SIZE_LARGE_BYTES: u64 = 100 * 1024 * 1024; // 100MB

/// 内存限制
pub const MAX_MEMORY_MB: u64 = 512;
pub const MAX_MEMORY_LARGE_MB: u64 = 2048;

/// 并发限制
pub const MAX_CONCURRENT_REQUESTS: usize = 10;
pub const MAX_CONCURRENT_TOOLS: usize = 5;

/// 日志配置
pub const LOG_LEVEL_DEFAULT: &str = "info";
pub const LOG_LEVEL_DEBUG: &str = "debug";
pub const LOG_LEVEL_TRACE: &str = "trace";

/// 路径分隔符
pub const PATH_SEPARATOR: char = std::path::MAIN_SEPARATOR;

/// 换行符
pub const LINE_ENDING: &str = if cfg!(windows) { "\r\n" } else { "\n" };
