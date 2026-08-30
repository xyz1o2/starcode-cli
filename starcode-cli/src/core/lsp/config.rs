/// LSP配置

use serde::{Deserialize, Serialize};

/// LSP服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspConfig {
    /// 是否启用
    pub enabled: bool,
    /// 服务器命令
    pub command: String,
    /// 命令参数
    pub args: Vec<String>,
    /// 根目录
    pub root_uri: Option<String>,
    /// 初始化选项
    pub initialization_options: Option<serde_json::Value>,
    /// 设置
    pub settings: Option<serde_json::Value>,
    /// 文件关联
    pub file_associations: Vec<String>,
    /// 是否启用诊断
    pub diagnostics_enabled: bool,
    /// 是否启用代码补全
    pub completion_enabled: bool,
    /// 是否启用悬停提示
    pub hover_enabled: bool,
    /// 是否启用定义跳转
    pub definition_enabled: bool,
    /// 是否启用引用查找
    pub references_enabled: bool,
    /// 是否启用格式化
    pub formatting_enabled: bool,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: String::new(),
            args: Vec::new(),
            root_uri: None,
            initialization_options: None,
            settings: None,
            file_associations: Vec::new(),
            diagnostics_enabled: true,
            completion_enabled: true,
            hover_enabled: true,
            definition_enabled: true,
            references_enabled: true,
            formatting_enabled: true,
        }
    }
}

impl LspConfig {
    /// 创建Rust Analyzer配置
    pub fn rust_analyzer() -> Self {
        Self {
            enabled: true,
            command: "rust-analyzer".to_string(),
            args: Vec::new(),
            root_uri: None,
            initialization_options: None,
            settings: None,
            file_associations: vec!["rs".to_string()],
            diagnostics_enabled: true,
            completion_enabled: true,
            hover_enabled: true,
            definition_enabled: true,
            references_enabled: true,
            formatting_enabled: true,
        }
    }

    /// 创建TypeScript配置
    pub fn typescript() -> Self {
        Self {
            enabled: true,
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            root_uri: None,
            initialization_options: None,
            settings: None,
            file_associations: vec!["ts".to_string(), "tsx".to_string(), "js".to_string(), "jsx".to_string()],
            diagnostics_enabled: true,
            completion_enabled: true,
            hover_enabled: true,
            definition_enabled: true,
            references_enabled: true,
            formatting_enabled: true,
        }
    }

    /// 创建Python配置
    pub fn python() -> Self {
        Self {
            enabled: true,
            command: "pylsp".to_string(),
            args: Vec::new(),
            root_uri: None,
            initialization_options: None,
            settings: None,
            file_associations: vec!["py".to_string()],
            diagnostics_enabled: true,
            completion_enabled: true,
            hover_enabled: true,
            definition_enabled: true,
            references_enabled: true,
            formatting_enabled: true,
        }
    }

    /// 创建Go配置
    pub fn gopls() -> Self {
        Self {
            enabled: true,
            command: "gopls".to_string(),
            args: Vec::new(),
            root_uri: None,
            initialization_options: None,
            settings: None,
            file_associations: vec!["go".to_string()],
            diagnostics_enabled: true,
            completion_enabled: true,
            hover_enabled: true,
            definition_enabled: true,
            references_enabled: true,
            formatting_enabled: true,
        }
    }
}
