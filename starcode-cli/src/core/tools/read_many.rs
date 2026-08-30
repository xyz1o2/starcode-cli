/// Read Many Files Tool - AI-Friendly Batch Reading
///
/// 核心价值：减少工具调用次数，一次读取多个文件
///
/// 场景：
/// - 查看多个相关文件（如所有 .rs 文件）
/// - 读取目录下的所有文件
/// - 对比多个文件内容
use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, LocationType, ToolInvocation, ToolLocation,
    ToolResult as CoreToolResult,
};
use crate::core::utils::file_utils::{detect_encoding_simple, format_file_size, is_binary_file};
use crate::core::utils::paths::resolve_path;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;

#[derive(Clone)]
pub struct ReadManyFilesTool {
    config: Arc<crate::core::config::Config>,
    global_state: Arc<GlobalState>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ReadManyParams {
    pub file_paths: Vec<String>,
    pub skip_binary: Option<bool>,
    pub max_size_per_file: Option<u64>,
    pub truncate_lines: Option<usize>,
}

pub struct ReadManyToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: ReadManyParams,
    global_state: Arc<GlobalState>,
}

impl ReadManyToolInvocation {
    pub fn new(config: Arc<crate::core::config::Config>, params: ReadManyParams, global_state: Arc<GlobalState>) -> Self {
        Self { config, params, global_state }
    }
}

/// 文件操作结果
#[derive(Debug, Clone)]
pub struct FileOpResult {
    pub success: bool,
    pub file_path: String,
    pub content: Option<String>,
    pub error: Option<String>,
    pub metadata: Option<FileMetadata>,
}

/// 文件元数据
#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub size: u64,
    pub lines: usize,
    pub is_binary: bool,
    pub encoding: String,
}

impl FileOpResult {
    pub fn success(file_path: String, content: String, metadata: FileMetadata) -> Self {
        Self {
            success: true,
            file_path,
            content: Some(content),
            error: None,
            metadata: Some(metadata),
        }
    }

    pub fn error(file_path: String, error: String) -> Self {
        Self {
            success: false,
            file_path,
            content: None,
            error: Some(error),
            metadata: None,
        }
    }
}

impl ReadManyFilesTool {
    pub fn new(config: Arc<crate::core::config::Config>, global_state: Arc<GlobalState>) -> Self {
        Self { config, global_state }
    }

    pub async fn read_many(
        &self,
        paths: Vec<String>,
        skip_binary: Option<bool>,
        max_size: Option<u64>,
        truncate_lines: Option<usize>,
    ) -> Result<CoreToolResult, String> {
        let mut results = Vec::new();

        for path_str in paths {
            let path_result = resolve_path(&path_str);

            match path_result {
                Ok(path) => {
                    let res = self
                        .read_single_file(&path, &path_str, skip_binary, max_size, truncate_lines)
                        .await;
                    results.push(res);
                }
                Err(e) => {
                    results.push(FileOpResult::error(path_str, e));
                }
            }
        }

        Ok(CoreToolResult {
            llm_content: None,
            return_display: None,
            output: format_batch_results(&results),
            error: None,
            data: None,
        })
    }

    async fn read_single_file(
        &self,
        path: &PathBuf,
        original_path: &str,
        skip_binary: Option<bool>,
        max_size: Option<u64>,
        truncate_lines: Option<usize>,
    ) -> FileOpResult {
        // 1. Check existence
        if !path.exists() {
            return FileOpResult::error(original_path.to_string(), "File not found".to_string());
        }

        // 2. Check metadata
        let metadata = match fs::metadata(path).await {
            Ok(m) => m,
            Err(e) => return FileOpResult::error(original_path.to_string(), e.to_string()),
        };

        if metadata.is_dir() {
            return FileOpResult::error(
                original_path.to_string(),
                "Path is a directory".to_string(),
            );
        }

        let size = metadata.len();
        if let Some(max) = max_size {
            if size > max {
                return FileOpResult::error(
                    original_path.to_string(),
                    format!(
                        "File too large: {} (max: {})",
                        format_file_size(size),
                        format_file_size(max)
                    ),
                );
            }
        }

        // 3. Read content
        // Use read to get bytes first to check binary
        match fs::read(path).await {
            Ok(bytes) => {
                let is_binary = is_binary_file(&bytes);
                let encoding = detect_encoding_simple(&bytes);

                if is_binary && skip_binary.unwrap_or(true) {
                    return FileOpResult::error(
                        original_path.to_string(),
                        "Binary file skipped".to_string(),
                    );
                }

                // Try to convert to string
                match String::from_utf8(bytes) {
                    Ok(content) => {
                        // Normalize CRLF -> LF for consistent tool behaviour
                        let content = super::edit::normalize_line_endings(&content);
                        let line_count = content.lines().count();

                        let final_content = if let Some(limit) = truncate_lines {
                            if line_count > limit {
                                let lines: Vec<&str> = content.lines().take(limit).collect();
                                let mut s = lines.join("\n");
                                s.push_str(&format!(
                                    "\n... (truncated, {} lines total)",
                                    line_count
                                ));
                                s
                            } else {
                                content
                            }
                        } else {
                            content
                        };

                        FileOpResult::success(
                            original_path.to_string(),
                            final_content,
                            FileMetadata {
                                size,
                                lines: line_count,
                                is_binary,
                                encoding,
                            },
                        )
                    }
                    Err(_) => {
                        // Binary or non-UTF8
                        FileOpResult::error(
                            original_path.to_string(),
                            "Could not decode as UTF-8".to_string(),
                        )
                    }
                }
            }
            Err(e) => FileOpResult::error(original_path.to_string(), e.to_string()),
        }
    }
}

/// 为 AI 生成友好的批量结果摘要
pub fn format_batch_results(results: &[FileOpResult]) -> String {
    let total = results.len();
    let success_count = results.iter().filter(|r| r.success).count();
    let failed_count = total - success_count;

    let mut output = String::new();

    // Summary
    if failed_count == 0 {
        output.push_str(&format!("Read {} files\n\n", total));
    } else {
        output.push_str(&format!(
            "Read {} files: {} succeeded, {} failed\n\n",
            total, success_count, failed_count
        ));
    }

    // Successful files
    let successful: Vec<_> = results.iter().filter(|r| r.success).collect();
    if !successful.is_empty() {
        output.push_str("Successful:\n");
        for result in &successful {
            if let Some(ref metadata) = result.metadata {
                output.push_str(&format!(
                    "  - {} ({}, {} lines)\n",
                    result.file_path,
                    format_file_size(metadata.size),
                    metadata.lines
                ));
            } else {
                output.push_str(&format!("  - {}\n", result.file_path));
            }
        }
        output.push('\n');
    }

    // Failed files
    let failed: Vec<_> = results.iter().filter(|r| !r.success).collect();
    if !failed.is_empty() {
        output.push_str("Failed:\n");
        for result in failed {
            output.push_str(&format!(
                "  - {}\n    Reason: {}\n",
                result.file_path,
                result
                    .error
                    .as_ref()
                    .unwrap_or(&"Unknown error".to_string())
            ));
        }
        output.push('\n');
    }

    // File contents
    if !successful.is_empty() {
        output.push_str("--- File contents ---\n\n");
        for result in successful {
            output.push_str(&format!("File: {}\n", result.file_path));
            output.push_str("```\n");
            output.push_str(result.content.as_deref().unwrap_or(""));
            output.push_str("\n```\n\n");
        }
    }

    output.trim_end().to_string()
}

impl ToolInvocation for ReadManyToolInvocation {
    fn get_description(&self) -> String {
        format!("Read {} files", self.params.file_paths.len())
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        self.params
            .file_paths
            .iter()
            .map(|p| ToolLocation {
                path: std::path::PathBuf::from(p),
                location_type: LocationType::Read,
            })
            .collect()
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let tool = ReadManyFilesTool::new(config, self.global_state.clone());
        let paths = self.params.file_paths.clone();
        let skip = self.params.skip_binary;
        let max = self.params.max_size_per_file;
        let trunc = self.params.truncate_lines;
        let global_state = self.global_state.clone();

        Box::pin(async move {
            let result = tool
                .read_many(paths.clone(), skip, max, trunc)
                .await
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            // ── 更新 read_file_state：避免后续编辑被 [edit_file_not_read] 拦截 ──
            // 从 output 中解析成功读取的文件路径，更新全局状态
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            {
                let mut state = global_state.read_file_state.write().await;
                for path_str in &paths {
                    if let Ok(abs) = resolve_path(path_str) {
                        let abs_str = abs.to_string_lossy().to_string();
                        // 只用当前时间作为占位时间戳（read_many 不返回文件元数据）
                        state.entry(abs_str).or_insert(ReadFileState {
                            content: String::new(), // read_many 批量内容，单独缓存意义不大
                            timestamp: now,
                            file_system_timestamp: now,
                        });
                    }
                }
            }

            Ok(result)
        })
    }
}

impl BaseDeclarativeTool for ReadManyFilesTool {
    fn name(&self) -> &str {
        "read_many_files"
    }

    fn display_name(&self) -> &str {
        "Read Many Files"
    }

    fn description(&self) -> &str {
        "Read multiple files at once. Use this to read 2+ files instead of calling Read multiple times."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of file paths to read"
                },
                "skip_binary": {
                    "type": "boolean",
                    "description": "Skip binary files (default: true)"
                },
                "max_size_per_file": {
                    "type": "integer",
                    "description": "Max bytes per file"
                },
                "truncate_lines": {
                    "type": "integer",
                    "description": "Truncate after N lines"
                }
            },
            "required": ["file_paths"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ReadManyParams = serde_json::from_value(params)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(Box::new(ReadManyToolInvocation::new(
            self.config.clone(),
            params,
            self.global_state.clone(),
        )))
    }
}
