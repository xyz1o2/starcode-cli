use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

/// 工具结果持久化配置
#[derive(Debug, Clone)]
pub struct ToolResultPersistConfig {
    /// 是否启用持久化
    pub enabled: bool,
    /// 持久化阈值（字符数）
    pub persist_threshold: usize,
    /// 预览大小（字节）
    pub preview_size: usize,
    /// 最大工具结果大小（字符数）
    pub max_result_size: usize,
}

impl Default for ToolResultPersistConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            persist_threshold: 50_000, // 50K字符
            preview_size: 2000,        // 2KB预览
            max_result_size: 500_000,  // 500K字符
        }
    }
}

impl ToolResultPersistConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_TOOL_RESULT_PERSIST_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let persist_threshold = std::env::var("STAR_TOOL_RESULT_PERSIST_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000);

        let preview_size = std::env::var("STAR_TOOL_RESULT_PREVIEW_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2000);

        let max_result_size = std::env::var("STAR_TOOL_RESULT_MAX_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500_000);

        Self {
            enabled,
            persist_threshold,
            preview_size,
            max_result_size,
        }
    }
}

/// 持久化的工具结果信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedToolResult {
    /// 文件路径
    pub filepath: String,
    /// 原始大小（字节）
    pub original_size: usize,
    /// 是否为JSON格式
    pub is_json: bool,
    /// 预览内容
    pub preview: String,
    /// 是否有更多内容
    pub has_more: bool,
}

/// 工具结果持久化管理器
pub struct ToolResultPersister {
    config: ToolResultPersistConfig,
    session_dir: PathBuf,
}

impl ToolResultPersister {
    pub fn new(session_dir: PathBuf) -> Self {
        let config = ToolResultPersistConfig::from_env();
        Self {
            config,
            session_dir,
        }
    }

    /// 获取工具结果目录
    fn tool_results_dir(&self) -> PathBuf {
        self.session_dir.join("tool-results")
    }

    /// 确保工具结果目录存在
    async fn ensure_dir(&self) -> std::io::Result<()> {
        let dir = self.tool_results_dir();
        if !dir.exists() {
            fs::create_dir_all(&dir).await?;
        }
        Ok(())
    }

    /// 获取工具结果文件路径
    fn get_result_path(&self, tool_use_id: &str, is_json: bool) -> PathBuf {
        let ext = if is_json { "json" } else { "txt" };
        self.tool_results_dir()
            .join(format!("{}.{}", tool_use_id, ext))
    }

    /// 生成预览内容
    fn generate_preview(&self, content: &str) -> (String, bool) {
        let preview_bytes = self.config.preview_size;
        if content.len() <= preview_bytes {
            (content.to_string(), false)
        } else {
            let preview: String = content.chars().take(preview_bytes).collect();
            (preview, true)
        }
    }

    /// 持久化工具结果
    pub async fn persist(
        &self,
        tool_use_id: &str,
        content: &str,
    ) -> Result<PersistedToolResult, String> {
        if !self.config.enabled {
            return Err("Persistence disabled".to_string());
        }

        // 检查大小是否超过阈值
        if content.len() < self.config.persist_threshold {
            return Err("Content below threshold".to_string());
        }

        // 截断到最大大小
        let truncated: String = if content.len() > self.config.max_result_size {
            content.chars().take(self.config.max_result_size).collect()
        } else {
            content.to_string()
        };

        self.ensure_dir().await.map_err(|e| e.to_string())?;

        let is_json = truncated.starts_with('{') || truncated.starts_with('[');
        let filepath = self.get_result_path(tool_use_id, is_json);

        // 检查文件是否已存在（避免重复写入）
        if !filepath.exists() {
            fs::write(&filepath, &truncated)
                .await
                .map_err(|e| e.to_string())?;
        }

        let (preview, has_more) = self.generate_preview(&truncated);

        Ok(PersistedToolResult {
            filepath: filepath.to_string_lossy().to_string(),
            original_size: content.len(),
            is_json,
            preview,
            has_more,
        })
    }

    /// 读取持久化的工具结果
    pub async fn read(&self, tool_use_id: &str) -> Result<String, String> {
        // 尝试JSON和TXT两种格式
        for ext in &["json", "txt"] {
            let filepath = self
                .tool_results_dir()
                .join(format!("{}.{}", tool_use_id, ext));
            if filepath.exists() {
                return fs::read_to_string(&filepath)
                    .await
                    .map_err(|e| e.to_string());
            }
        }
        Err("Tool result not found".to_string())
    }

    /// 构建大结果的消息
    pub fn build_large_result_message(&self, result: &PersistedToolResult) -> String {
        let mut message = String::from("<persisted-output>\n");
        message.push_str(&format!(
            "Output too large ({}). Full output saved to: {}\n\n",
            format_size(result.original_size),
            result.filepath
        ));
        message.push_str(&format!(
            "Preview (first {}):\n",
            format_size(self.config.preview_size)
        ));
        message.push_str(&result.preview);
        if result.has_more {
            message.push_str("\n...\n");
        }
        message.push_str("\n</persisted-output>");
        message
    }

    /// 应用工具结果预算
    pub fn apply_budget(&self, content: &str) -> String {
        if content.len() <= self.config.max_result_size {
            return content.to_string();
        }

        let truncated: String = content.chars().take(self.config.max_result_size).collect();
        format!(
            "{}\n\n... (truncated, original size: {})",
            truncated,
            format_size(content.len())
        )
    }
}

/// 格式化文件大小
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500B");
        assert_eq!(format_size(1500), "1.5KB");
        assert_eq!(format_size(1500000), "1.4MB");
    }

    #[test]
    fn test_generate_preview() {
        let config = ToolResultPersistConfig::default();
        let persister = ToolResultPersister {
            config,
            session_dir: PathBuf::from("/tmp/test"),
        };

        let (preview, has_more) = persister.generate_preview("short content");
        assert_eq!(preview, "short content");
        assert!(!has_more);

        let long_content = "x".repeat(5000);
        let (preview, has_more) = persister.generate_preview(&long_content);
        assert_eq!(preview.len(), 2000);
        assert!(has_more);
    }
}
