/// LSP服务器实例

use serde::{Deserialize, Serialize};
use super::config::LspConfig;
use super::diagnostic::DiagnosticRegistry;

/// 服务器状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServerState {
    /// 未启动
    NotStarted,
    /// 启动中
    Starting,
    /// 运行中
    Running,
    /// 错误
    Error(String),
    /// 已停止
    Stopped,
}

/// LSP服务器实例
pub struct LspServerInstance {
    /// 实例ID
    pub id: String,
    /// 配置
    pub config: LspConfig,
    /// 状态
    pub state: ServerState,
    /// 诊断注册表
    pub diagnostics: DiagnosticRegistry,
    /// 打开的文件
    pub open_files: std::collections::HashSet<String>,
    /// 启动时间
    pub started_at: Option<i64>,
}

impl LspServerInstance {
    /// 创建新的服务器实例
    pub fn new(id: String, config: LspConfig) -> Self {
        Self {
            id,
            config,
            state: ServerState::NotStarted,
            diagnostics: DiagnosticRegistry::new(),
            open_files: std::collections::HashSet::new(),
            started_at: None,
        }
    }

    /// 启动服务器
    pub fn start(&mut self) -> Result<(), LspInstanceError> {
        if self.state == ServerState::Running {
            return Err(LspInstanceError::AlreadyRunning);
        }

        self.state = ServerState::Starting;
        self.started_at = Some(chrono::Utc::now().timestamp());

        // TODO: 实际启动LSP服务器进程
        // 这里简化处理，直接设置为运行状态
        self.state = ServerState::Running;

        Ok(())
    }

    /// 停止服务器
    pub fn stop(&mut self) {
        self.state = ServerState::Stopped;
        self.open_files.clear();
    }

    /// 打开文件
    pub fn open_file(&mut self, file_path: &str) {
        self.open_files.insert(file_path.to_string());
    }

    /// 关闭文件
    pub fn close_file(&mut self, file_path: &str) {
        self.open_files.remove(file_path);
    }

    /// 检查文件是否打开
    pub fn is_file_open(&self, file_path: &str) -> bool {
        self.open_files.contains(file_path)
    }

    /// 获取状态
    pub fn state(&self) -> &ServerState {
        &self.state
    }

    /// 检查是否运行中
    pub fn is_running(&self) -> bool {
        self.state == ServerState::Running
    }
}

/// LSP实例错误
#[derive(Debug)]
pub enum LspInstanceError {
    /// 已经在运行
    AlreadyRunning,
    /// 启动失败
    StartFailed(String),
    /// 配置错误
    ConfigError(String),
}

impl std::fmt::Display for LspInstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LspInstanceError::AlreadyRunning => write!(f, "LSP server is already running"),
            LspInstanceError::StartFailed(e) => write!(f, "Failed to start LSP server: {}", e),
            LspInstanceError::ConfigError(e) => write!(f, "LSP config error: {}", e),
        }
    }
}

impl std::error::Error for LspInstanceError {}
