// ============================================================================
// 文本编辑工具模块
// ============================================================================
//
// 提供各种文本编辑能力：
// - SmartEdit: 智能代码编辑（多策略自动回退）
// - TextEditor: 基础文本编辑
// - MorphEditor: 交互式编辑
//
// 设计思路：
// 1. 使用策略模式，每种编辑策略独立实现
// 2. SmartEdit 协调多个策略，自动选择最佳策略
// 3. 提供统一的 EditResult 返回格式

pub mod smart_edit;
pub mod strategies;
pub mod tool_integration;

/// 编辑结果
#[derive(Debug, Clone)]
pub struct EditResult {
    /// 是否成功
    pub success: bool,
    /// 新内容
    pub new_content: String,
    /// 匹配次数
    pub occurrences: usize,
    /// 使用的策略名称
    pub strategy: String,
    /// 详细信息（用于日志/调试）
    pub details: Option<String>,
}

impl EditResult {
    pub fn success(new_content: String, occurrences: usize, strategy: &str) -> Self {
        Self {
            success: true,
            new_content,
            occurrences,
            strategy: strategy.to_string(),
            details: None,
        }
    }

    pub fn failure(strategy: &str, reason: String) -> Self {
        Self {
            success: false,
            new_content: String::new(),
            occurrences: 0,
            strategy: strategy.to_string(),
            details: Some(reason),
        }
    }

    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }
}

/// 编辑上下文
#[derive(Debug, Clone)]
pub struct EditContext {
    /// 文件路径
    pub file_path: String,
    /// 原始内容
    pub content: String,
    /// 要替换的旧字符串
    pub old_string: String,
    /// 新字符串
    pub new_string: String,
}

impl EditContext {
    pub fn new(file_path: String, content: String, old_string: String, new_string: String) -> Self {
        Self {
            file_path,
            content,
            old_string,
            new_string,
        }
    }
}
