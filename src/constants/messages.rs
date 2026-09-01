/// 消息常量

/// 系统消息
pub const MSG_WELCOME: &str = "Welcome to StarCode CLI!";
pub const MSG_GOODBYE: &str = "Goodbye! Thank you for using StarCode CLI.";
pub const MSG_LOADING: &str = "Loading...";
pub const MSG_PROCESSING: &str = "Processing...";
pub const MSG_THINKING: &str = "Thinking...";
pub const MSG_GENERATING: &str = "Generating response...";
pub const MSG_SEARCHING: &str = "Searching...";
pub const MSG_ANALYZING: &str = "Analyzing...";
pub const MSG_EXECUTING: &str = "Executing...";
pub const MSG_COMPLETED: &str = "Completed!";
pub const MSG_FAILED: &str = "Failed!";
pub const MSG_CANCELLED: &str = "Cancelled.";
pub const MSG_TIMEOUT: &str = "Operation timed out.";

/// 错误消息
pub const ERR_MSG_INTERNAL: &str = "An internal error occurred. Please try again.";
pub const ERR_MSG_NETWORK: &str = "Network error. Please check your connection.";
pub const ERR_MSG_AUTH: &str = "Authentication failed. Please check your credentials.";
pub const ERR_MSG_PERMISSION: &str = "Permission denied. You don't have access to this resource.";
pub const ERR_MSG_NOT_FOUND: &str = "Resource not found.";
pub const ERR_MSG_RATE_LIMIT: &str = "Rate limit exceeded. Please wait before trying again.";
pub const ERR_MSG_TIMEOUT: &str = "Request timed out. Please try again.";
pub const ERR_MSG_INVALID_INPUT: &str = "Invalid input. Please check your input and try again.";
pub const ERR_MSG_FILE_READ: &str = "Failed to read file.";
pub const ERR_MSG_FILE_WRITE: &str = "Failed to write file.";
pub const ERR_MSG_TOOL_EXEC: &str = "Tool execution failed.";
pub const ERR_MSG_API_KEY: &str = "API key is missing or invalid.";

/// 确认消息
pub const CONFIRM_DELETE: &str = "Are you sure you want to delete this?";
pub const CONFIRM_EXIT: &str = "Are you sure you want to exit?";
pub const CONFIRM_RESET: &str = "Are you sure you want to reset?";
pub const CONFIRM_OVERWRITE: &str = "File already exists. Overwrite?";
pub const CONFIRM_CONTINUE: &str = "Do you want to continue?";

/// 提示消息
pub const PROMPT_INPUT: &str = "Enter your message:";
pub const PROMPT_FILE_PATH: &str = "Enter file path:";
pub const PROMPT_CONFIRM: &str = "Confirm? (y/n):";
pub const PROMPT_SELECT: &str = "Select an option:";
pub const PROMPT_PASSWORD: &str = "Enter password:";

/// 状态消息
pub const STATUS_IDLE: &str = "Idle";
pub const STATUS_BUSY: &str = "Busy";
pub const STATUS_WAITING: &str = "Waiting";
pub const STATUS_ERROR: &str = "Error";
pub const STATUS_CONNECTED: &str = "Connected";
pub const STATUS_DISCONNECTED: &str = "Disconnected";

/// 进度消息
pub const PROGRESS_STARTING: &str = "Starting...";
pub const PROGRESS_IN_PROGRESS: &str = "In progress...";
pub const PROGRESS_COMPLETING: &str = "Completing...";
pub const PROGRESS_DONE: &str = "Done!";
