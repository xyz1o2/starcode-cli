/// SSH会话管理系统
/// 
/// 对标claude-code-main的src/ssh/
/// SSH会话创建、认证和管理

pub mod auth;
pub mod deploy;
pub mod probe;
pub mod session;

pub use auth::SSHAuthProxy;
pub use deploy::SSHDeploy;
pub use probe::SSHProbe;
pub use session::{SSHSession, SSHSessionManager};

use serde::{Deserialize, Serialize};

/// SSH配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SSHConfig {
    /// 主机
    pub host: String,
    /// 端口
    pub port: u16,
    /// 用户名
    pub username: String,
    /// 认证方式
    pub auth_method: SSHAuthMethod,
    /// 超时（秒）
    pub timeout_secs: u64,
    /// 保持连接
    pub keep_alive: bool,
}

/// SSH认证方式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SSHAuthMethod {
    /// 密码认证
    Password(String),
    /// 密钥认证
    KeyFile(String),
    /// 代理认证
    Agent,
}

/// SSH连接状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SSHConnectionStatus {
    /// 断开连接
    Disconnected,
    /// 连接中
    Connecting,
    /// 已连接
    Connected,
    /// 错误
    Error(String),
}

/// SSH执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SSHExecResult {
    /// 命令
    pub command: String,
    /// 退出码
    pub exit_code: i32,
    /// 标准输出
    pub stdout: String,
    /// 标准错误
    pub stderr: String,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
}
