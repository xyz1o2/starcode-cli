/// 文档生成器

use super::{DocType, DocResult, DocError};
use super::prompts::DocPrompts;

/// 文档生成器
pub struct DocGenerator {
    /// 提示词
    prompts: DocPrompts,
}

impl DocGenerator {
    /// 创建新的文档生成器
    pub fn new() -> Self {
        Self {
            prompts: DocPrompts::new(),
        }
    }

    /// 生成文档
    pub fn generate(&self, doc_type: &DocType, project_path: &str) -> Result<DocResult, DocError> {
        let content = match doc_type {
            DocType::Readme => self.generate_readme(project_path)?,
            DocType::Api => self.generate_api_docs(project_path)?,
            DocType::Architecture => self.generate_architecture_docs(project_path)?,
            DocType::Contributing => self.generate_contributing_docs(project_path)?,
            DocType::Changelog => self.generate_changelog(project_path)?,
            DocType::Custom(name) => self.generate_custom_docs(project_path, name)?,
        };

        let file_path = format!("{}/{}.md", project_path, self.get_filename(doc_type));

        Ok(DocResult {
            doc_type: doc_type.clone(),
            file_path,
            content,
            generated_at: chrono::Utc::now().timestamp(),
            word_count: content.split_whitespace().count() as u32,
        })
    }

    /// 生成README
    fn generate_readme(&self, project_path: &str) -> Result<String, DocError> {
        // 分析项目结构
        let project_name = std::path::Path::new(project_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Project");

        let content = format!(
            r#"# {}

## Overview

This is a software project.

## Installation

```bash
# Add installation instructions here
```

## Usage

```bash
# Add usage examples here
```

## Development

```bash
# Add development setup here
```

## License

See LICENSE file for details.
"#,
            project_name
        );

        Ok(content)
    }

    /// 生成API文档
    fn generate_api_docs(&self, project_path: &str) -> Result<String, DocError> {
        Ok("# API Documentation\n\nAPI documentation will be generated here.\n".to_string())
    }

    /// 生成架构文档
    fn generate_architecture_docs(&self, project_path: &str) -> Result<String, DocError> {
        Ok("# Architecture\n\nArchitecture documentation will be generated here.\n".to_string())
    }

    /// 生成贡献指南
    fn generate_contributing_docs(&self, project_path: &str) -> Result<String, DocError> {
        Ok(r#"# Contributing

## How to Contribute

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request

## Code Style

Please follow the existing code style.

## Testing

Run all tests before submitting.
"#.to_string())
    }

    /// 生成变更日志
    fn generate_changelog(&self, project_path: &str) -> Result<String, DocError> {
        Ok(r#"# Changelog

## [Unreleased]

### Added
- Initial release

### Changed
- None

### Fixed
- None
"#.to_string())
    }

    /// 生成自定义文档
    fn generate_custom_docs(&self, project_path: &str, name: &str) -> Result<String, DocError> {
        Ok(format!("# {}\n\nCustom documentation for {}.\n", name, name))
    }

    /// 获取文件名
    fn get_filename(&self, doc_type: &DocType) -> &str {
        match doc_type {
            DocType::Readme => "README",
            DocType::Api => "API",
            DocType::Architecture => "ARCHITECTURE",
            DocType::Contributing => "CONTRIBUTING",
            DocType::Changelog => "CHANGELOG",
            DocType::Custom(name) => name,
        }
    }
}
