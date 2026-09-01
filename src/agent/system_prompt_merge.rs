//! System Prompt 多级合并模块
//!
//! 对标 Claude Code 的 system-prompt.mdx：
//! - 五级优先级：Override > Coordinator > Agent > Custom > Default
//! - 静态区/动态区分离
//! - CLAUDE.md 多级合并
//! - 缓存策略

use serde::{Serialize, Deserialize};
use std::path::{Path, PathBuf};

/// System Prompt 优先级
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PromptPriority {
    /// 默认提示词
    Default = 0,
    /// 用户自定义
    Custom = 1,
    /// Agent 专用
    Agent = 2,
    /// Coordinator 专用
    Coordinator = 3,
    /// 最高优先级覆盖
    Override = 4,
}

/// Prompt 段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSection {
    pub priority: PromptPriority,
    pub name: String,
    pub content: String,
    pub cacheable: bool,
    pub cache_ttl_secs: u64,
}

/// 合并后的 System Prompt
#[derive(Debug, Clone)]
pub struct MergedSystemPrompt {
    /// 静态部分（不变的）
    pub static_part: String,
    /// 动态部分（每次可能变化）
    pub dynamic_part: String,
    /// 完整 prompt
    pub full: String,
    /// 是否命中缓存
    pub cache_hit: bool,
}

/// System Prompt 合并器
pub struct SystemPromptMerger {
    sections: Vec<PromptSection>,
    claudemd_contents: Vec<(PathBuf, String)>,
    boundary_marker: String,
}

impl SystemPromptMerger {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
            claudemd_contents: Vec::new(),
            boundary_marker: "===SYSTEM_PROMPT_DYNAMIC_BOUNDARY===".to_string(),
        }
    }

    /// 添加 prompt 段
    pub fn add_section(&mut self, section: PromptSection) {
        self.sections.push(section);
    }

    /// 添加 CLAUDE.md 内容
    pub fn add_claudemd(&mut self, path: PathBuf, content: String) {
        self.claudemd_contents.push((path, content));
    }

    /// 合并所有 prompt 段
    pub fn merge(&self) -> MergedSystemPrompt {
        // 按优先级排序
        let mut sorted = self.sections.clone();
        sorted.sort_by(|a, b| a.priority.cmp(&b.priority));

        let mut static_parts = Vec::new();
        let mut dynamic_parts = Vec::new();

        // 添加 CLAUDE.md 内容（作为 Custom 优先级）
        for (path, content) in &self.claudemd_contents {
            let section = format!(
                "# Project context from {}\n{}",
                path.display(),
                content
            );
            static_parts.push(section);
        }

        // 分离静态和动态部分
        for section in &sorted {
            if section.cacheable {
                static_parts.push(section.content.clone());
            } else {
                dynamic_parts.push(section.content.clone());
            }
        }

        let static_part = static_parts.join("\n\n");
        let dynamic_part = dynamic_parts.join("\n\n");
        let full = if dynamic_part.is_empty() {
            static_part.clone()
        } else {
            format!("{}\n\n{}\n\n{}", static_part, self.boundary_marker, dynamic_part)
        };

        MergedSystemPrompt {
            static_part,
            dynamic_part,
            full,
            cache_hit: false,
        }
    }

    /// 从项目目录加载 CLAUDE.md 文件
    pub fn load_claudemd_files(&mut self, project_root: &Path) -> Result<(), String> {
        // 项目根目录
        let root_claudemd = project_root.join("CLAUDE.md");
        if root_claudemd.exists() {
            let content = std::fs::read_to_string(&root_claudemd)
                .map_err(|e| format!("Failed to read CLAUDE.md: {}", e))?;
            self.claudemd_contents.push((root_claudemd, content));
        }

        // .claude/ 目录下的配置
        let claude_dir = project_root.join(".claude");
        if claude_dir.exists() {
            // .claude/CLAUDE.md
            let dir_claudemd = claude_dir.join("CLAUDE.md");
            if dir_claudemd.exists() {
                if let Ok(content) = std::fs::read_to_string(&dir_claudemd) {
                    self.claudemd_contents.push((dir_claudemd, content));
                }
            }

            // .claude/commands/ 下的自定义命令
            let commands_dir = claude_dir.join("commands");
            if commands_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&commands_dir) {
                    for entry in entries.flatten() {
                        if entry.path().extension().map(|e| e == "md").unwrap_or(false) {
                            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                                self.claudemd_contents.push((entry.path(), content));
                            }
                        }
                    }
                }
            }
        }

        // 用户全局 CLAUDE.md
        if let Some(home) = dirs::home_dir() {
            let global_claudemd = home.join(".claude").join("CLAUDE.md");
            if global_claudemd.exists() {
                if let Ok(content) = std::fs::read_to_string(&global_claudemd) {
                    self.claudemd_contents.push((global_claudemd, content));
                }
            }
        }

        Ok(())
    }

    /// 构建默认 system prompt
    pub fn build_default_prompt(&self) -> String {
        r#"You are an AI coding assistant. You help users with software engineering tasks including:
- Reading and understanding code
- Writing and editing code
- Debugging and fixing issues
- Running commands and tests
- Searching for information
- Planning and implementing features

Always follow best practices, write clean code, and explain your reasoning when asked."#
            .to_string()
    }

    /// 构建安全策略 prompt
    pub fn build_security_prompt(&self) -> String {
        r#"## Security Guidelines
- Never expose secrets, API keys, or credentials
- Never commit sensitive data to version control
- Always validate user inputs
- Follow the principle of least privilege
- Prefer read-only operations when possible"#
            .to_string()
    }
}

/// Prompt 缓存管理
pub struct PromptCache {
    cache: std::collections::HashMap<String, CachedPrompt>,
    max_entries: usize,
}

#[derive(Clone)]
struct CachedPrompt {
    content: String,
    created_at: std::time::Instant,
    ttl: std::time::Duration,
    hit_count: u64,
}

impl PromptCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: std::collections::HashMap::new(),
            max_entries,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<String> {
        if let Some(entry) = self.cache.get_mut(key) {
            if entry.created_at.elapsed() < entry.ttl {
                entry.hit_count += 1;
                return Some(entry.content.clone());
            } else {
                self.cache.remove(key);
            }
        }
        None
    }

    pub fn put(&mut self, key: String, content: String, ttl: std::time::Duration) {
        if self.cache.len() >= self.max_entries {
            // 驱逐最旧的条目
            if let Some(oldest_key) = self
                .cache
                .iter()
                .min_by_key(|(_, v)| v.created_at)
                .map(|(k, _)| k.clone())
            {
                self.cache.remove(&oldest_key);
            }
        }

        self.cache.insert(
            key,
            CachedPrompt {
                content,
                created_at: std::time::Instant::now(),
                ttl,
                hit_count: 0,
            },
        );
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn stats(&self) -> (usize, u64) {
        let total_hits: u64 = self.cache.values().map(|v| v.hit_count).sum();
        (self.cache.len(), total_hits)
    }
}
