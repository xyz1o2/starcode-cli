use crate::core::confirmation_bus::MessageBus;
use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::file_utils::{process_file_read_blocking, ProcessedFileReadResult};
use crate::core::utils::paths::{make_relative, shorten_path};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileToolParams {
    #[serde(rename = "file_path")]
    pub file_path: Option<String>,
    #[serde(rename = "file_paths")]
    pub file_paths: Option<Vec<String>>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

/// 读一份"和 `Edit`/`Write` 比对时读到的东西完全一致"的原文。
///
/// 换行必须按 `\r\n → \n` 归一，否则 Windows 上 CRLF 文件永远比对不相等，
/// 那道"内容没变就别拦"的逃生口就又废了。读失败退回空串：调用方只需要一个
/// 保守的基线，而 `Edit` 对空基线的处理是"当成没有基线"，不会误放行。
async fn read_current_content(path: &std::path::Path) -> String {
    match crate::core::utils::file_utils::read_file_with_encoding_async(path).await {
        Ok(content) => super::edit::normalize_line_endings(&content),
        Err(_) => String::new(),
    }
}

/// 把一次成功读取登记进两份 read state。
///
/// 两份都要写，因为 `Edit`/`Write` 查的是 `read_file_state`（`edit.rs`、
/// `write_file.rs`），而 `MultiEdit`/`NotebookEdit` 查的是 `execution_state`。
///
/// `abs_path` 必须由调用方用 [`crate::core::utils::paths::read_state_key`] 算：
/// 这里只负责写，不负责规范化，否则写入端和查询端的键会分叉，明明读过也报
/// `[edit_file_not_read]`。
pub(crate) async fn record_file_read(
    global_state: &Arc<GlobalState>,
    abs_path: &str,
    content: String,
    timestamp: u128,
    file_system_timestamp: u128,
) {
    {
        let mut state = global_state.read_file_state.write().await;
        state.insert(
            abs_path.to_string(),
            ReadFileState {
                content: content.clone(),
                timestamp,
                file_system_timestamp,
            },
        );
    }

    {
        use crate::core::state::FileSnapshot;
        let hash = crate::core::utils::paths::sha256_hex(content.as_bytes());
        let snapshot = FileSnapshot {
            content,
            timestamp,
            hash,
        };
        let mut exec_state = global_state.execution_state.write().await;
        exec_state.mark_file_read(abs_path.to_string(), snapshot);
    }
}

pub struct ReadFileToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: ReadFileToolParams,
    global_state: Arc<GlobalState>,
    resolved_paths: Vec<PathBuf>,
}

impl ReadFileToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: ReadFileToolParams,
        _message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
    ) -> Self {
        let resolved_paths = if let Some(file_paths) = &params.file_paths {
            file_paths
                .iter()
                .map(|p| config.target_dir().join(p))
                .collect()
        } else if let Some(file_path) = &params.file_path {
            vec![config.target_dir().join(file_path)]
        } else {
            vec![]
        };

        Self {
            config,
            params,
            global_state,
            resolved_paths,
        }
    }
}

use crate::core::utils::normalization::{normalize_to_size, NormalizationConfig};
use serde_json::json;

impl ToolInvocation for ReadFileToolInvocation {
    fn get_description(&self) -> String {
        if self.resolved_paths.len() > 1 {
            format!("Read {} files", self.resolved_paths.len())
        } else if let Some(path) = self.resolved_paths.first() {
            let relative_path = make_relative(path, self.config.target_dir());
            shorten_path(&relative_path.to_string_lossy(), 80)
        } else {
            "Read file".to_string()
        }
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        self.resolved_paths
            .iter()
            .map(|path| ToolLocation {
                path: path.clone(),
                location_type: crate::core::tools::tools::LocationType::Read,
            })
            .collect()
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let paths = self.resolved_paths.clone();

        Box::pin(async move {
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    for path in &paths {
                        let is_trusted = tf.is_path_trusted(path).unwrap_or(false);
                        if !is_trusted {
                            let path_clone = path.clone();
                            let config_clone = config.clone();
                            return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                                 confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                                 title: "Untrusted Folder".to_string(),
                                 prompt: format!("Security: Path {:?} is not in a trusted folder. Do you want to proceed?", path),
                                 on_confirm: std::sync::Arc::new(move |outcome, _feedback| {
                                     if let crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave = outcome {
                                         if let Some(tf) = config_clone.trusted_folders() {
                                             let folder_to_trust = if path_clone.is_dir() {
                                                  path_clone.clone()
                                              } else {
                                                  path_clone.parent().unwrap_or(&path_clone).to_path_buf()
                                              };
                                             let _ = tf.set_trust_level(&folder_to_trust, crate::core::config::trusted_folders::TrustLevel::TrustFolder);
                                         }
                                     }
                                 }),
                             }));
                        }
                    }
                }
            }
            Ok(None)
        })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let _config = self.config.clone();
        let params = self.params.clone();
        let resolved_paths = self.resolved_paths.clone();
        let global_state = self.global_state.clone();

        Box::pin(async move {
            // 如果有多个文件路径，批量读取
            if resolved_paths.len() > 1 {
                let mut results = Vec::new();
                for resolved_path in &resolved_paths {
                    let result = tokio::task::spawn_blocking({
                        let path = resolved_path.clone();
                        let offset = params.offset;
                        let limit = params.limit;
                        move || process_file_read_blocking(&path, offset, limit)
                    })
                    .await;

                    match result {
                        Ok(Ok(file_result)) => {
                            results.push((resolved_path.clone(), file_result));
                        }
                        Ok(Err(e)) => {
                            results.push((
                                resolved_path.clone(),
                                ProcessedFileReadResult {
                                    llm_content: String::new(),
                                    return_display: String::new(),
                                    error: Some(e.to_string()),
                                    error_type: Some(ToolErrorType::Unknown),
                                    is_truncated: Some(false),
                                    original_line_count: Some(0),
                                    lines_shown: Some((0, 0)),
                                },
                            ));
                        }
                        Err(e) => {
                            results.push((
                                resolved_path.clone(),
                                ProcessedFileReadResult {
                                    llm_content: String::new(),
                                    return_display: String::new(),
                                    error: Some(e.to_string()),
                                    error_type: Some(ToolErrorType::Unknown),
                                    is_truncated: Some(false),
                                    original_line_count: Some(0),
                                    lines_shown: Some((0, 0)),
                                },
                            ));
                        }
                    }
                }

                // 批量读**同样要登记 read state**。
                //
                // 之前这条分支直接 return，一个文件都没记。于是
                // `Read(file_paths=[a, b])` → 内容完整返回给模型 → `Edit(a)` 被
                // `[edit_file_not_read]` 拦下，100% 复现。更糟的是
                // `Read`/`Glob` 标着 read-only，命中 5 分钟工具结果缓存时
                // `execute()` 根本不执行，这里也就再也不会被跑到 ——
                // 同一个批次参数可以无限次"读到了内容却始终没被登记"。
                for (path, file_result) in &results {
                    if file_result.error.is_some() {
                        continue;
                    }
                    let abs_path = crate::core::utils::paths::read_state_key(path);
                    let timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis();
                    let file_system_timestamp = {
                        let probe = path.clone();
                        tokio::task::spawn_blocking(move || {
                            std::fs::metadata(&probe)
                                .and_then(|m| m.modified())
                                .unwrap_or(SystemTime::UNIX_EPOCH)
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                        })
                        .await
                        .unwrap_or_default()
                    };
                    // 同上：登记磁盘原文，而不是 `--- path ---` 拼出来的批处理转储。
                    record_file_read(
                        &global_state,
                        &abs_path,
                        read_current_content(path).await,
                        timestamp,
                        file_system_timestamp,
                    )
                    .await;
                }

                // 合并结果
                let mut combined_output = String::new();
                let mut has_errors = false;
                for (path, result) in &results {
                    if let Some(error) = &result.error {
                        combined_output.push_str(&format!(
                            "Error reading {}: {}\n",
                            path.display(),
                            error
                        ));
                        has_errors = true;
                    } else {
                        combined_output.push_str(&format!("--- {} ---\n", path.display()));
                        combined_output.push_str(&result.llm_content);
                        combined_output.push_str("\n\n");
                    }
                }

                Ok(ToolResult {
                    llm_content: Some(combined_output.clone()),
                    return_display: Some(combined_output.clone()),
                    output: combined_output,
                    error: if has_errors {
                        Some(crate::core::tools::tools::ToolError {
                            error_type: "batch_read_error".to_string(),
                            message: "Some files had errors".to_string(),
                        })
                    } else {
                        None
                    },
                    data: None,
                })
            } else {
                // 单个文件读取
                let resolved_path = resolved_paths.first().cloned().unwrap_or_default();
                let resolved_path_for_closure = resolved_path.clone();
                let result = tokio::task::spawn_blocking(move || {
                    process_file_read_blocking(
                        &resolved_path_for_closure,
                        params.offset,
                        params.limit,
                    )
                })
                .await;

                let mut result = match result {
                    Ok(res) => res.map_err(|e| {
                        Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
                            as Box<dyn std::error::Error>
                    })?,
                    Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
                };

                // Normalize CRLF -> LF for consistent tool behaviour
                {
                    let normalized = super::edit::normalize_line_endings(&result.llm_content);
                    result.llm_content = normalized;
                }

                if let Some(error) = &result.error {
                    return Ok(ToolResult {
                        llm_content: Some(result.llm_content.clone()),
                        return_display: Some(result.return_display.clone()),
                        output: result.return_display.clone(),
                        error: Some(crate::core::tools::tools::ToolError {
                            error_type: result
                                .error_type
                                .clone()
                                .map(|t| t.to_string())
                                .unwrap_or_else(|| ToolErrorType::Unknown.to_string()),
                            message: error.clone(),
                        }),
                        data: None,
                    });
                }

                // Update ReadFileState
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();

                // Get file system timestamp (blocking, but fast metadata)
                let file_system_timestamp = {
                    let path = resolved_path.clone();
                    tokio::task::spawn_blocking(move || {
                        std::fs::metadata(&path)
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH)
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                    })
                    .await
                    .unwrap_or_default()
                };

                // read_file_state 的键必须和 Edit/Write 查表时用同一个函数算
                // （见 paths::read_state_key），否则读过的文件仍会被
                // [edit_file_not_read] 拦下。
                let abs_path = crate::core::utils::paths::read_state_key(&resolved_path);

                // 登记的必须是**磁盘原文**，不能是给模型看的那份。
                //
                // `process_file_read_blocking` 返回的是 `lines.join("\n")`：行尾
                // 换行没了、被截断时只剩一个窗口、prompt-injection 标签也被剥掉了。
                // 而 `Edit`/`Write` 判断"文件在读之后有没有被改"时，是拿
                // `read_file_with_encoding_async` 的**原始全文**去和登记内容比：
                //
                //     current_content != file_state.content
                //
                // 两者结构上永远不可能相等（光是行尾换行就足够），于是那道
                // "mtime 变了但内容没变就别拦"的逃生口**一次都进不去** ——
                // 只要构建、格式化器、`git checkout` 或上一次 bash 编辑动过 mtime，
                // 就报 `[edit_file_modified]`。这就是那个"时不时"冒出来的红色报错。
                let raw_content = read_current_content(&resolved_path).await;

                record_file_read(
                    &global_state,
                    &abs_path,
                    raw_content,
                    timestamp,
                    file_system_timestamp,
                )
                .await;

                let mut llm_content =
                    if result.is_truncated.unwrap_or(false) {
                        let (start, end) = result.lines_shown.unwrap_or((0, 0));
                        let total = result.original_line_count.unwrap_or(0);
                        let next_offset = end; // 0-based offset for next read

                        format!(
                    "The file content has been truncated (showing lines {}-{} of {} total).\n\
                     To read more, use: read_file(file_path=\"{}\", offset={}, limit={})\n\n\
                     --- FILE CONTENT (truncated) ---\n\
                     {}",
                    start, end, total,
                    resolved_path.display(), end, total - end,
                    result.llm_content
                )
                    } else {
                        result.llm_content.clone()
                    };

                // ── AST symbol overview injection (P2) ────────────────────────────
                {
                    let file_ext = resolved_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");
                    if let Some(overview) =
                        crate::core::context::symbols_for_read::build_symbol_overview(
                            &result.llm_content,
                            file_ext,
                        )
                    {
                        llm_content.push_str(&overview);
                    }
                }

                // Apply normalization to very large files even if not truncated by line count (e.g. huge lines)
                // or if the content is just generally too large for context window
                // 80KB limit for read_file output (increased to reduce truncation)
                let normalized_value = normalize_to_size(
                    json!(llm_content),
                    Some(NormalizationConfig {
                        target_size: 80 * 1024,
                        ..Default::default()
                    }),
                );

                if let Some(s) = normalized_value.as_str() {
                    llm_content = s.to_string();
                }

                Ok(ToolResult {
                    llm_content: Some(llm_content.clone()),
                    return_display: Some(llm_content.clone()),
                    output: llm_content,
                    error: None,
                    data: None,
                })
            }
        })
    }
}

pub struct ReadFileTool {
    config: Arc<crate::core::config::Config>,
    message_bus: Arc<MessageBus>,
    global_state: Arc<GlobalState>,
    override_name: Option<String>,
}

impl ReadFileTool {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            message_bus,
            global_state,
            override_name: None,
        }
    }

    pub fn new_with_name(
        config: Arc<crate::core::config::Config>,
        message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
        name: String,
    ) -> Self {
        Self {
            config,
            message_bus,
            global_state,
            override_name: Some(name),
        }
    }

    pub fn name(&self) -> &str {
        self.override_name.as_deref().unwrap_or("Read")
    }

    pub fn display_name(&self) -> &str {
        "ReadFile"
    }

    pub fn description(&self) -> &str {
        "Reads and returns the content of a specified file. If the file is large, the content will be truncated. The tool's response will clearly indicate if truncation has occurred and will provide details on how to read more of the file using the 'offset' and 'limit' parameters. Handles text, images (PNG, JPG, GIF, WEBP, SVG, BMP), and PDF files. For text files, it can read specific line ranges. IMPORTANT: Prefer reading the entire file at once (without offset/limit) to avoid multiple tool calls."
    }

    pub fn kind(&self) -> Kind {
        Kind::Read
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "properties": {
                "file_path": {
                    "description": "The path to the file to read.",
                    "type": "string"
                },
                "file_paths": {
                    "description": "Optional: List of file paths to read in batch mode.",
                    "type": "array",
                    "items": {
                        "type": "string"
                    }
                },
                "offset": {
                    "description": "Optional: For text files, the 0-based line number to start reading from. Requires 'limit' to be set. Use for paginating through large files. PREFER reading the entire file without offset/limit when possible.",
                    "type": "number"
                },
                "limit": {
                    "description": "Optional: For text files, maximum number of lines to read. Use with 'offset' to paginate through large files. If omitted, reads the entire file (if feasible, up to a default limit). AVOID setting small limits — prefer reading the full file.",
                    "type": "number"
                }
            },
            "required": [],
            "type": "object"
        })
    }

    pub fn validate_tool_params(&self, params: &ReadFileToolParams) -> Result<(), String> {
        if let Some(file_path) = &params.file_path {
            if file_path.trim().is_empty() {
                return Err("The 'file_path' parameter must be non-empty.".to_string());
            }
        } else if let Some(file_paths) = &params.file_paths {
            if file_paths.is_empty() {
                return Err("The 'file_paths' parameter must be non-empty.".to_string());
            }
            for file_path in file_paths {
                if file_path.trim().is_empty() {
                    return Err(
                        "The 'file_paths' parameter must not contain empty strings.".to_string()
                    );
                }
            }
        } else {
            return Err("Either 'file_path' or 'file_paths' parameter is required.".to_string());
        }

        if let Some(limit) = params.limit {
            if limit == 0 {
                return Err("Limit must be a positive number".to_string());
            }
        }

        Ok(())
    }

    pub fn build(&self, params: ReadFileToolParams) -> Box<dyn ToolInvocation> {
        Box::new(ReadFileToolInvocation::new(
            self.config.clone(),
            params,
            self.message_bus.clone(),
            self.global_state.clone(),
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
        ))
    }
}

impl BaseDeclarativeTool for ReadFileTool {
    fn name(&self) -> &str {
        ReadFileTool::name(self)
    }

    fn display_name(&self) -> &str {
        ReadFileTool::display_name(self)
    }

    fn description(&self) -> &str {
        ReadFileTool::description(self)
    }

    fn kind(&self) -> Kind {
        ReadFileTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        ReadFileTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ReadFileToolParams = serde_json::from_value(params)?;
        self.validate_tool_params(&params)
            .map_err(|e| e.to_string())?;
        Ok(self.build(params))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("star-readfile-{}-{}", tag, std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    async fn test_config(dir: &std::path::Path) -> Arc<crate::core::config::Config> {
        let mut parameters = crate::core::config::ConfigParameters::default();
        parameters.model = "test-model".to_string();
        parameters.core_tools = Some(Vec::new());
        parameters.mcp_enabled = Some(false);
        parameters.enable_agents = Some(false);
        let mut config = crate::core::config::Config::new(parameters);
        config.cwd = dir.to_path_buf();
        config
            .initialize()
            .await
            .expect("test config must initialize");
        Arc::new(config)
    }

    fn test_bus() -> Arc<crate::core::confirmation_bus::MessageBus> {
        Arc::new(crate::core::confirmation_bus::MessageBus::new(
            Default::default(),
            false,
        ))
    }

    /// 回归：批量 `Read` 也要登记 read state。
    ///
    /// 之前这条分支一个文件都不记，于是 `Read(file_paths=[a, b])` 完整返回内容
    /// 之后，紧跟着的 `Edit(a)` 100% 被 `[edit_file_not_read]` 拦下。
    #[tokio::test]
    async fn a_batch_read_records_every_file_it_returned() {
        let dir = temp_dir("batch");
        let a = dir.join("a.rs");
        let b = dir.join("b.rs");
        std::fs::write(&a, "fn a() {}\n").unwrap();
        std::fs::write(&b, "fn b() {}\n").unwrap();

        let config = test_config(&dir).await;
        let global_state = Arc::new(GlobalState::new());
        let tool = ReadFileTool::new(config, test_bus(), global_state.clone());
        let result = tool
            .create_invocation(serde_json::json!({
                "file_paths": [a.to_string_lossy(), b.to_string_lossy()],
            }))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap();
        assert!(result.error.is_none(), "批量读必须成功：{result:?}");

        let read_state = global_state.read_file_state.read().await;
        for path in [&a, &b] {
            let key = crate::core::utils::paths::read_state_key(path);
            assert!(
                read_state.contains_key(&key),
                "批量读返回了 {} 的内容却没登记，Edit 会被 [edit_file_not_read] 拦下",
                path.display()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 登记的内容必须和磁盘原文一致。
    ///
    /// 这是 `Edit`/`Write` 那道"mtime 变了但内容没变就别拦"的逃生口的前提：
    /// 它们拿此刻读到的**原文**和登记内容比。登记成给模型看的
    /// `lines.join("\n")`（行尾换行没了、可能被截断、标签被剥掉）就永远比不相等，
    /// 逃生口一次都进不去，只要文件被谁碰过 mtime 就报 `[edit_file_modified]`。
    #[tokio::test]
    async fn the_recorded_content_is_the_verbatim_file_content() {
        let dir = temp_dir("content");
        let a = dir.join("only.rs");
        let body = "fn only() {}\n";
        std::fs::write(&a, body).unwrap();

        let config = test_config(&dir).await;
        let global_state = Arc::new(GlobalState::new());
        let tool = ReadFileTool::new(config, test_bus(), global_state.clone());
        tool.create_invocation(serde_json::json!({ "file_path": a.to_string_lossy() }))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap();

        let key = crate::core::utils::paths::read_state_key(&a);
        let read_state = global_state.read_file_state.read().await;
        assert_eq!(
            read_state.get(&key).map(|s| s.content.as_str()),
            Some(body),
            "登记的必须是磁盘原文（含行尾换行），否则内容比对永远判'变了'"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 截断读取登记的也必须是**全文**：模型只看到窗口，但后续 `Edit` 的内容比对
    /// 是拿全文比的，登记窗口等于登记了一个必然对不上的基线。
    #[tokio::test]
    async fn a_truncated_read_still_records_the_whole_file() {
        let dir = temp_dir("truncated");
        let a = dir.join("long.rs");
        let body: String = (0..500).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&a, &body).unwrap();

        let config = test_config(&dir).await;
        let global_state = Arc::new(GlobalState::new());
        let tool = ReadFileTool::new(config, test_bus(), global_state.clone());
        let result = tool
            .create_invocation(serde_json::json!({ "file_path": a.to_string_lossy(), "limit": 10 }))
            .unwrap()
            .execute(None, None)
            .await
            .unwrap();
        assert!(result.error.is_none());

        let key = crate::core::utils::paths::read_state_key(&a);
        let read_state = global_state.read_file_state.read().await;
        assert_eq!(
            read_state.get(&key).map(|s| s.content.as_str()),
            Some(body.as_str()),
            "截断只该影响给模型看的窗口，不能影响登记的比对基线"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
