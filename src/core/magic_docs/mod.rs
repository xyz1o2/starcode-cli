/// Magic Docs文档自动生成系统
/// 
/// 对标claude-code-main的src/services/MagicDocs/
/// 自动为项目生成和更新文档

pub mod generator;
pub mod parser;
pub mod prompts;

pub use generator::DocGenerator;
pub use parser::CodeParser;
pub use prompts::DocPrompts;

use serde::{Deserialize, Serialize};

/// 文档类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DocType {
    /// README
    Readme,
    /// API文档
    Api,
    /// 架构文档
    Architecture,
    /// 贡献指南
    Contributing,
    /// 变更日志
    Changelog,
    /// 自定义
    Custom(String),
}

/// 文档配置
#[derive(Debug, Clone)]
pub struct DocConfig {
    /// 是否启用
    pub enabled: bool,
    /// 输出目录
    pub output_dir: String,
    /// 文档类型
    pub doc_types: Vec<DocType>,
    /// 是否自动更新
    pub auto_update: bool,
    /// 更新间隔（秒）
    pub update_interval_secs: u64,
}

impl Default for DocConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output_dir: "docs".to_string(),
            doc_types: vec![DocType::Readme],
            auto_update: false,
            update_interval_secs: 86400, // 24小时
        }
    }
}

impl DocConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_MAGIC_DOCS_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let output_dir = std::env::var("STAR_MAGIC_DOCS_OUTPUT")
            .unwrap_or_else(|_| "docs".to_string());

        Self {
            enabled,
            output_dir,
            doc_types: vec![DocType::Readme],
            auto_update: false,
            update_interval_secs: 86400,
        }
    }
}

/// 文档生成结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocResult {
    /// 文档类型
    pub doc_type: DocType,
    /// 文件路径
    pub file_path: String,
    /// 内容
    pub content: String,
    /// 生成时间
    pub generated_at: i64,
    /// 字数
    pub word_count: u32,
}

/// Magic Docs管理器
pub struct MagicDocsManager {
    /// 配置
    config: DocConfig,
    /// 文档生成器
    generator: DocGenerator,
    /// 代码解析器
    parser: CodeParser,
    /// 生成历史
    history: Vec<DocResult>,
}

impl MagicDocsManager {
    /// 创建新的Magic Docs管理器
    pub fn new(config: DocConfig) -> Self {
        Self {
            config,
            generator: DocGenerator::new(),
            parser: CodeParser::new(),
            history: Vec::new(),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(DocConfig::from_env())
    }

    /// 生成文档
    pub fn generate_docs(&mut self, project_path: &str) -> Result<Vec<DocResult>, DocError> {
        if !self.config.enabled {
            return Err(DocError::NotEnabled);
        }

        let mut results = Vec::new();

        for doc_type in &self.config.doc_types {
            let result = self.generator.generate(doc_type, project_path)?;
            results.push(result);
        }

        // 保存历史
        self.history.extend(results.clone());

        Ok(results)
    }

    /// 获取生成历史
    pub fn get_history(&self) -> &[DocResult] {
        &self.history
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

/// Doc错误
#[derive(Debug)]
pub enum DocError {
    /// 未启用
    NotEnabled,
    /// 生成错误
    GenerationError(String),
    /// 解析错误
    ParseError(String),
    /// IO错误
    IoError(String),
}

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocError::NotEnabled => write!(f, "Magic Docs is not enabled"),
            DocError::GenerationError(e) => write!(f, "Doc generation error: {}", e),
            DocError::ParseError(e) => write!(f, "Code parse error: {}", e),
            DocError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for DocError {}
