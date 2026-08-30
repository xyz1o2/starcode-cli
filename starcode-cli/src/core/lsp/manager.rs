/// LSP服务器管理器
/// 
/// 对标claude-code-main的src/services/lsp/
/// 管理多个LSP服务器实例，支持被动反馈

use std::collections::HashMap;
use super::config::LspConfig;
use super::instance::{LspServerInstance, ServerState};
use super::diagnostic::Diagnostic;
use super::LspLanguage;

/// 被动反馈
#[derive(Debug, Clone)]
pub struct PassiveFeedback {
    /// 文件路径
    pub file: String,
    /// 反馈类型
    pub feedback_type: FeedbackType,
    /// 消息
    pub message: String,
    /// 严重性
    pub severity: FeedbackSeverity,
    /// 位置
    pub location: Option<FeedbackLocation>,
}

/// 反馈类型
#[derive(Debug, Clone)]
pub enum FeedbackType {
    /// 诊断
    Diagnostic,
    /// 代码动作
    CodeAction,
    /// 格式化建议
    Formatting,
    /// 导入建议
    ImportSuggestion,
    /// 类型提示
    TypeHint,
}

/// 反馈严重性
#[derive(Debug, Clone)]
pub enum FeedbackSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// 反馈位置
#[derive(Debug, Clone)]
pub struct FeedbackLocation {
    pub line: u32,
    pub column: u32,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
}

/// LSP服务器管理器
pub struct LspServerManager {
    /// 服务器实例
    instances: HashMap<String, LspServerInstance>,
    /// 语言到实例的映射
    language_map: HashMap<LspLanguage, String>,
    /// 文件到语言的映射
    file_language_map: HashMap<String, LspLanguage>,
    /// 被动反馈队列
    feedback_queue: Vec<PassiveFeedback>,
}

impl LspServerManager {
    /// 创建新的LSP服务器管理器
    pub fn new() -> Self {
        Self {
            instances: HashMap::new(),
            language_map: HashMap::new(),
            file_language_map: HashMap::new(),
            feedback_queue: Vec::new(),
        }
    }

    /// 注册LSP服务器
    pub fn register_server(&mut self, language: LspLanguage, config: LspConfig) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let instance = LspServerInstance::new(id.clone(), config);
        
        self.instances.insert(id.clone(), instance);
        self.language_map.insert(language, id.clone());
        
        id
    }

    /// 启动服务器
    pub fn start_server(&mut self, server_id: &str) -> Result<(), LspManagerError> {
        let instance = self.instances.get_mut(server_id)
            .ok_or(LspManagerError::ServerNotFound(server_id.to_string()))?;
        
        instance.start()
            .map_err(|e| LspManagerError::StartFailed(e.to_string()))
    }

    /// 停止服务器
    pub fn stop_server(&mut self, server_id: &str) {
        if let Some(instance) = self.instances.get_mut(server_id) {
            instance.stop();
        }
    }

    /// 打开文件
    pub fn open_file(&mut self, file_path: &str) -> Result<(), LspManagerError> {
        // 检测语言
        let language = self.detect_language(file_path);
        
        // 找到对应的服务器
        let server_id = self.language_map.get(&language)
            .ok_or(LspManagerError::NoServerForLanguage(language.name().to_string()))?;
        
        let instance = self.instances.get_mut(server_id)
            .ok_or(LspManagerError::ServerNotFound(server_id.clone()))?;
        
        if !instance.is_running() {
            return Err(LspManagerError::ServerNotRunning(server_id.clone()));
        }
        
        instance.open_file(file_path);
        self.file_language_map.insert(file_path.to_string(), language);
        
        Ok(())
    }

    /// 关闭文件
    pub fn close_file(&mut self, file_path: &str) {
        if let Some(language) = self.file_language_map.remove(file_path) {
            if let Some(server_id) = self.language_map.get(&language) {
                if let Some(instance) = self.instances.get_mut(server_id) {
                    instance.close_file(file_path);
                }
            }
        }
    }

    /// 获取文件的诊断
    pub fn get_diagnostics(&self, file_path: &str) -> Vec<&Diagnostic> {
        if let Some(language) = self.file_language_map.get(file_path) {
            if let Some(server_id) = self.language_map.get(language) {
                if let Some(instance) = self.instances.get(server_id) {
                    return instance.diagnostics.get_diagnostics(file_path);
                }
            }
        }
        Vec::new()
    }

    /// 获取所有诊断
    pub fn get_all_diagnostics(&self) -> Vec<&Diagnostic> {
        self.instances.values()
            .flat_map(|instance| instance.diagnostics.get_all_diagnostics())
            .collect()
    }

    /// 添加被动反馈
    pub fn add_passive_feedback(&mut self, feedback: PassiveFeedback) {
        self.feedback_queue.push(feedback);
    }

    /// 获取被动反馈
    pub fn get_passive_feedback(&self) -> &[PassiveFeedback] {
        &self.feedback_queue
    }

    /// 清空被动反馈
    pub fn clear_passive_feedback(&mut self) {
        self.feedback_queue.clear();
    }

    /// 生成被动反馈
    pub fn generate_passive_feedback(&mut self, file_path: &str) {
        let diagnostics = self.get_diagnostics(file_path);
        
        for diag in diagnostics {
            let severity = match diag.severity {
                super::diagnostic::DiagnosticSeverity::Error => FeedbackSeverity::Error,
                super::diagnostic::DiagnosticSeverity::Warning => FeedbackSeverity::Warning,
                super::diagnostic::DiagnosticSeverity::Information => FeedbackSeverity::Info,
                super::diagnostic::DiagnosticSeverity::Hint => FeedbackSeverity::Hint,
            };

            let feedback = PassiveFeedback {
                file: file_path.to_string(),
                feedback_type: FeedbackType::Diagnostic,
                message: diag.message.clone(),
                severity,
                location: Some(FeedbackLocation {
                    line: diag.line,
                    column: diag.column,
                    end_line: diag.end_line,
                    end_column: diag.end_column,
                }),
            };

            self.feedback_queue.push(feedback);
        }
    }

    /// 检测文件语言
    fn detect_language(&self, file_path: &str) -> LspLanguage {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        
        LspLanguage::from_extension(ext)
    }

    /// 获取服务器实例
    pub fn get_instance(&self, server_id: &str) -> Option<&LspServerInstance> {
        self.instances.get(server_id)
    }

    /// 获取所有服务器实例
    pub fn get_all_instances(&self) -> Vec<&LspServerInstance> {
        self.instances.values().collect()
    }

    /// 获取运行中的服务器数
    pub fn running_servers(&self) -> usize {
        self.instances.values()
            .filter(|instance| instance.is_running())
            .count()
    }
}

/// LSP管理器错误
#[derive(Debug)]
pub enum LspManagerError {
    /// 服务器未找到
    ServerNotFound(String),
    /// 启动失败
    StartFailed(String),
    /// 服务器未运行
    ServerNotRunning(String),
    /// 没有对应语言的服务器
    NoServerForLanguage(String),
}

impl std::fmt::Display for LspManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LspManagerError::ServerNotFound(id) => write!(f, "LSP server not found: {}", id),
            LspManagerError::StartFailed(e) => write!(f, "Failed to start LSP server: {}", e),
            LspManagerError::ServerNotRunning(id) => write!(f, "LSP server not running: {}", id),
            LspManagerError::NoServerForLanguage(lang) => write!(f, "No LSP server for language: {}", lang),
        }
    }
}

impl std::error::Error for LspManagerError {}
