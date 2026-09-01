/// Hook执行器
/// 
/// 对标claude-code-main的src/utils/hooks/exec*.ts
/// 提供不同类型的Hook执行器

pub mod prompt_hook;
pub mod agent_hook;
pub mod http_hook;

pub use prompt_hook::PromptHookExecutor;
pub use agent_hook::AgentHookExecutor;
pub use http_hook::HttpHookExecutor;

use serde::{Deserialize, Serialize};

/// Hook类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HookType {
    /// 命令Hook
    Command,
    /// 提示词Hook
    Prompt,
    /// Agent Hook
    Agent,
    /// HTTP Hook
    Http,
    /// 函数Hook
    Function,
}

/// Hook定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// Hook ID
    pub id: String,
    /// Hook名称
    pub name: String,
    /// Hook类型
    pub hook_type: HookType,
    /// Hook事件
    pub event: String,
    /// 匹配器
    pub matcher: Option<String>,
    /// 命令/提示词/URL
    pub command: String,
    /// 超时（秒）
    pub timeout: Option<u64>,
    /// 是否阻塞
    pub blocking: bool,
    /// 来源
    pub source: String,
}

/// Hook执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    /// Hook ID
    pub hook_id: String,
    /// 是否成功
    pub success: bool,
    /// 输出
    pub output: Option<String>,
    /// 错误
    pub error: Option<String>,
    /// 退出码
    pub exit_code: Option<i32>,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
    /// 是否阻止继续
    pub prevent_continuation: bool,
    /// 停止原因
    pub stop_reason: Option<String>,
}

/// Hook执行器trait
#[async_trait::async_trait]
pub trait HookExecutor: Send + Sync {
    /// 执行Hook
    async fn execute(
        &self,
        hook: &HookDefinition,
        input: &str,
        signal: Option<tokio_util::sync::CancellationToken>,
    ) -> Result<HookResult, HookError>;
    
    /// 检查是否支持此Hook类型
    fn supports(&self, hook_type: &HookType) -> bool;
}

/// Hook错误
#[derive(Debug)]
pub enum HookError {
    /// 超时
    Timeout,
    /// 执行失败
    ExecutionFailed(String),
    /// 配置错误
    ConfigError(String),
    /// 权限错误
    PermissionError(String),
    /// 网络错误
    NetworkError(String),
}

impl std::fmt::Display for HookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookError::Timeout => write!(f, "Hook execution timed out"),
            HookError::ExecutionFailed(e) => write!(f, "Hook execution failed: {}", e),
            HookError::ConfigError(e) => write!(f, "Hook config error: {}", e),
            HookError::PermissionError(e) => write!(f, "Hook permission error: {}", e),
            HookError::NetworkError(e) => write!(f, "Hook network error: {}", e),
        }
    }
}

impl std::error::Error for HookError {}
