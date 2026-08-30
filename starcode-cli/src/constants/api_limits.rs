/// API限制常量

/// 最大token数
pub const MAX_TOKENS_DEFAULT: u32 = 8192;
pub const MAX_TOKENS_LARGE: u32 = 16384;
pub const MAX_TOKENS_EXTRA_LARGE: u32 = 32768;

/// 最大消息数
pub const MAX_MESSAGES_DEFAULT: usize = 100;
pub const MAX_MESSAGES_LARGE: usize = 500;

/// 最大工具调用数
pub const MAX_TOOL_CALLS_PER_TURN: usize = 10;
pub const MAX_TOOL_CALLS_TOTAL: usize = 100;

/// 最大输入长度
pub const MAX_INPUT_LENGTH: usize = 100000;
pub const MAX_INPUT_LENGTH_SHORT: usize = 10000;

/// API超时（秒）
pub const API_TIMEOUT_DEFAULT: u64 = 120;
pub const API_TIMEOUT_SHORT: u64 = 30;
pub const API_TIMEOUT_LONG: u64 = 300;

/// 重试配置
pub const MAX_RETRIES: u32 = 3;
pub const RETRY_DELAY_MS: u64 = 1000;
pub const RETRY_MAX_DELAY_MS: u64 = 10000;

/// 速率限制
pub const RATE_LIMIT_REQUESTS_PER_MINUTE: u32 = 60;
pub const RATE_LIMIT_TOKENS_PER_MINUTE: u32 = 100000;

/// 缓存配置
pub const CACHE_TTL_SECONDS: u64 = 3600;
pub const CACHE_MAX_SIZE: usize = 1000;

/// 上下文窗口大小
pub const CONTEXT_WINDOW_SMALL: u32 = 8192;
pub const CONTEXT_WINDOW_MEDIUM: u32 = 32768;
pub const CONTEXT_WINDOW_LARGE: u32 = 128000;
pub const CONTEXT_WINDOW_EXTRA_LARGE: u32 = 200000;
