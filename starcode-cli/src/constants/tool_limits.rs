/// 工具限制常量

/// 工具执行超时（毫秒）
pub const TOOL_TIMEOUT_DEFAULT: u64 = 30000;
pub const TOOL_TIMEOUT_SHORT: u64 = 5000;
pub const TOOL_TIMEOUT_LONG: u64 = 120000;
pub const TOOL_TIMEOUT_EXTRA_LONG: u64 = 300000;

/// 工具输出限制
pub const TOOL_OUTPUT_MAX_LENGTH: usize = 10000;
pub const TOOL_OUTPUT_MAX_LENGTH_LARGE: usize = 100000;
pub const TOOL_OUTPUT_TRUNCATE_LENGTH: usize = 5000;

/// 工具参数限制
pub const TOOL_ARGS_MAX_LENGTH: usize = 10000;
pub const TOOL_ARGS_MAX_COUNT: usize = 20;

/// 文件操作限制
pub const TOOL_FILE_MAX_SIZE: u64 = 10 * 1024 * 1024; // 10MB
pub const TOOL_FILE_MAX_LINES: usize = 10000;
pub const TOOL_FILE_MAX_LINE_LENGTH: usize = 10000;

/// 搜索限制
pub const TOOL_SEARCH_MAX_RESULTS: usize = 100;
pub const TOOL_SEARCH_MAX_CONTEXT_LINES: usize = 5;
pub const TOOL_SEARCH_MAX_PATTERN_LENGTH: usize = 1000;

/// Bash工具限制
pub const TOOL_BASH_MAX_COMMAND_LENGTH: usize = 10000;
pub const TOOL_BASH_MAX_OUTPUT_LENGTH: usize = 100000;
pub const TOOL_BASH_TIMEOUT: u64 = 60000;

/// 编辑工具限制
pub const TOOL_EDIT_MAX_CHANGES: usize = 100;
pub const TOOL_EDIT_MAX_DIFF_LENGTH: usize = 10000;

/// 读取工具限制
pub const TOOL_READ_MAX_LINES: usize = 1000;
pub const TOOL_READ_MAX_LINE_LENGTH: usize = 10000;
pub const TOOL_READ_CONTEXT_LINES: usize = 5;

/// 写入工具限制
pub const TOOL_WRITE_MAX_SIZE: u64 = 10 * 1024 * 1024; // 10MB
pub const TOOL_WRITE_MAX_LINES: usize = 100000;

/// Glob工具限制
pub const TOOL_GLOB_MAX_RESULTS: usize = 1000;
pub const TOOL_GLOB_MAX_PATTERN_LENGTH: usize = 1000;

/// Grep工具限制
pub const TOOL_GREP_MAX_RESULTS: usize = 1000;
pub const TOOL_GREP_MAX_MATCHES_PER_FILE: usize = 100;
pub const TOOL_GREP_MAX_FILE_SIZE: u64 = 10 * 1024 * 1024; // 10MB
