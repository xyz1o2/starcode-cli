use crate::core::confirmation_bus::MessageBus;
use crate::core::state::GlobalState;
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::diff_options::{create_patch, get_diff_stat};
use crate::core::tools::modifiable_tool::{ModifiableDeclarativeTool, ModifyContext};
use crate::core::tools::tools::{
    BaseDeclarativeTool, FileDiff, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::paths::{make_relative, shorten_path};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

fn full_rewrite_guard_enabled() -> bool {
    std::env::var("STAR_BLOCK_FULL_FILE_REWRITE")
        .ok()
        .map(|v| {
            let normalized = v.trim().to_lowercase();
            !(normalized == "0" || normalized == "false" || normalized == "off")
        })
        .unwrap_or(true)
}

fn rewrite_guard_min_lines() -> usize {
    std::env::var("STAR_WRITE_REWRITE_GUARD_MIN_LINES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(120)
        .max(20)
}

fn rewrite_guard_ratio_threshold() -> f64 {
    std::env::var("STAR_WRITE_REWRITE_GUARD_RATIO")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.70)
        .clamp(0.40, 0.95)
}

fn full_rewrite_auto_recover_enabled() -> bool {
    std::env::var("STAR_AUTO_RECOVER_FULL_FILE_REWRITE")
        .ok()
        .map(|v| {
            let normalized = v.trim().to_lowercase();
            !(normalized == "0" || normalized == "false" || normalized == "off")
        })
        .unwrap_or(true)
}

fn line_count(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count().max(1)
    }
}

fn changed_line_ratio(old_content: &str, new_content: &str) -> f64 {
    let mut changed_lines = 0usize;
    for change in similar::TextDiff::from_lines(old_content, new_content).iter_all_changes() {
        if !matches!(change.tag(), similar::ChangeTag::Equal) {
            changed_lines += 1;
        }
    }
    if old_content.is_empty() {
        return if changed_lines == 0 { 0.0 } else { 1.0 };
    }
    let base = line_count(old_content).max(1) as f64;
    changed_lines as f64 / base
}

fn full_rewrite_guard_message(
    file_path: &Path,
    original_content: &str,
    new_content: &str,
    is_new_file: bool,
) -> Option<String> {
    if is_new_file || !full_rewrite_guard_enabled() {
        return None;
    }

    let original_lines = line_count(original_content);
    if original_lines < rewrite_guard_min_lines() {
        return None;
    }

    let ratio = changed_line_ratio(original_content, new_content);
    let threshold = rewrite_guard_ratio_threshold();
    if ratio < threshold {
        return None;
    }

    Some(format!(
        "Write blocked [full_file_rewrite_blocked]: existing file '{}' would be replaced almost entirely (changed ratio {:.1}%, {} lines). \
Do not retry `write_file` with another full-file body. Read the file again and switch to `replace`, `smart_edit`, or `multi_edit` with targeted old/new hunks.",
        file_path.display(),
        ratio * 100.0,
        original_lines
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteFileToolParams {
    #[serde(rename = "file_path")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "modified_by_user")]
    pub modified_by_user: Option<bool>,
    #[serde(rename = "ai_proposed_content")]
    pub ai_proposed_content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GetCorrectedFileContentResult {
    pub original_content: String,
    pub corrected_content: String,
    pub file_exists: bool,
    pub error: Option<WriteFileError>,
    pub original_line_ending: super::edit::LineEnding,
}

#[derive(Debug, Clone)]
pub struct WriteFileError {
    pub message: String,
    pub code: Option<String>,
}

async fn write_content_atomically(
    resolved_path: &Path,
    corrected_content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = resolved_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let tmp_path = resolved_path.with_extension("star_tmp");
    tokio::fs::write(&tmp_path, corrected_content).await?;
    if let Err(_rename_err) = tokio::fs::rename(&tmp_path, resolved_path).await {
        tokio::fs::copy(&tmp_path, resolved_path).await?;
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }

    Ok(())
}

fn build_write_result(
    resolved_path: &Path,
    original_content: &str,
    corrected_content: &str,
    params: &WriteFileToolParams,
    success_message: String,
    recovery_strategy: Option<&str>,
) -> Result<ToolResult, Box<dyn std::error::Error>> {
    let file_name = resolved_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    let file_diff = create_patch(
        file_name,
        original_content,
        corrected_content,
        "Original",
        "Written",
    );

    let originally_proposed_content = params
        .ai_proposed_content
        .as_ref()
        .unwrap_or(&params.content);
    let diff_stat = get_diff_stat(
        file_name,
        original_content,
        originally_proposed_content,
        &params.content,
    );

    let mut llm_content = success_message.clone();
    if let Some(strategy) = recovery_strategy {
        llm_content.push_str(&format!(
            " Recovery strategy: {}. Prefer targeted edit tools for future edits to this file.",
            strategy
        ));
    }
    if params.modified_by_user.unwrap_or(false) {
        llm_content.push_str(&format!(
            " User modified the `content` to be: {}",
            params.content
        ));
    }

    let mut data = serde_json::json!({
        "diff": file_diff
    });
    if let Some(strategy) = recovery_strategy {
        data["recovery_strategy"] = serde_json::Value::String(strategy.to_string());
        data["auto_recovered"] = serde_json::Value::Bool(true);
    }

    Ok(ToolResult {
        llm_content: Some(llm_content),
        return_display: Some(serde_json::to_string(&FileDiff {
            file_diff: file_diff.clone(),
            file_name: file_name.to_string(),
            original_content: Some(original_content.to_string()),
            new_content: corrected_content.to_string(),
            diff_stat: Some(diff_stat),
        })?),
        output: success_message,
        error: None,
        data: Some(data),
    })
}

pub async fn get_corrected_file_content(
    _config: &Arc<crate::core::config::Config>,
    file_path: &Path,
    proposed_content: &str,
    _abort_signal: &tokio_util::sync::CancellationToken,
) -> Result<GetCorrectedFileContentResult, Box<dyn std::error::Error>> {
    let mut original_content = String::new();
    let mut _file_exists = false;
    let mut corrected_content = proposed_content.to_string();
    let mut error = None;
    let mut original_line_ending = super::edit::LineEnding::LF;

    match crate::core::utils::file_utils::read_file_with_encoding_async(file_path).await {
        Ok(content) => {
            original_line_ending = super::edit::detect_line_ending(&content);
            original_content = super::edit::normalize_line_endings(&content);
            _file_exists = true;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            _file_exists = false;
            original_content = String::new();
        }
        Err(e) => {
            _file_exists = true;
            error = Some(WriteFileError {
                message: e.to_string(),
                code: Some(format!("{:?}", e.kind())),
            });
            return Ok(GetCorrectedFileContentResult {
                original_content,
                corrected_content,
                file_exists: _file_exists,
                error,
                original_line_ending,
            });
        }
    }

    if _file_exists {
        corrected_content = super::edit::normalize_line_endings(proposed_content);
    } else {
        corrected_content = proposed_content.to_string();
    }

    Ok(GetCorrectedFileContentResult {
        original_content,
        corrected_content,
        file_exists: _file_exists,
        error,
        original_line_ending,
    })
}

pub struct WriteFileToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: WriteFileToolParams,
    global_state: Arc<GlobalState>,
    resolved_path: PathBuf,
}

impl WriteFileToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: WriteFileToolParams,
        _message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
    ) -> Self {
        let resolved_path = config.target_dir().join(&params.file_path);

        Self {
            config,
            params,
            global_state,
            resolved_path,
        }
    }
}

impl ToolInvocation for WriteFileToolInvocation {
    fn get_description(&self) -> String {
        let relative_path = make_relative(&self.resolved_path, self.config.target_dir());
        let short_path = shorten_path(&relative_path.to_string_lossy(), 40);

        format!("Write to {}", short_path)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: self.resolved_path.clone(),
            location_type: crate::core::tools::tools::LocationType::Write,
        }]
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
        let path = self.resolved_path.clone();

        Box::pin(async move {
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let is_trusted = tf.is_path_trusted(&path).unwrap_or(false);
                    if !is_trusted {
                        let path_clone = path.clone();
                        let config_clone = config.clone();
                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                             confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                             title: "Untrusted Folder".to_string(),
                             prompt: format!("Security: Path {:?} is not in a trusted folder. Do you want to proceed?", path),
                             on_confirm: std::sync::Arc::new(move |outcome| {
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
        let config = self.config.clone();
        let params = self.params.clone();
        let resolved_path = self.resolved_path.clone();
        let global_state = self.global_state.clone();
        Box::pin(async move {
            // File-history checkpoint: snapshot the file BEFORE we overwrite it.
            // track_edit is best-effort — failures must never block the write.
            // If the file is already backed up in the most recent snapshot
            // (e.g. speculative retries), track_edit is a no-op.
            {
                let msg_id = global_state.current_message_id().await;
                if let Err(e) = crate::utils::checkpoint_manager::track_edit(
                    &resolved_path,
                    msg_id,
                    Some("write_file"),
                    None, // session_id: per-cwd fallback, matches /undo and /rewind
                )
                .await
                {
                    log::warn!(
                        "FileHistory: track_edit failed for {}: {}",
                        resolved_path.display(),
                        e
                    );
                }
            }

            let mut existing_file_verified_for_overwrite = false;
            let strict_read_check = std::env::var("STAR_DISABLE_READ_CHECK")
                .map(|v| v.to_lowercase() != "true" && v != "1")
                .unwrap_or(true);

            if strict_read_check && tokio::fs::try_exists(&resolved_path).await.unwrap_or(false) {
                let abs_path = resolved_path
                    .canonicalize()
                    .unwrap_or(resolved_path.clone())
                    .to_string_lossy()
                    .to_string();

                let file_state = {
                    let read_state = global_state.read_file_state.read().await;
                    read_state.get(&abs_path).cloned()
                };

                if let Some(file_state) = file_state {
                    // If recorded timestamp was a fallback (0), skip strict modified check
                    if file_state.file_system_timestamp > 0 {
                        if let Ok(metadata) = tokio::fs::metadata(&resolved_path).await {
                            if let Ok(modified) = metadata.modified() {
                                let current_mtime = modified
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis();

                                // Use 2000ms buffer to account for filesystem timestamp granularity
                                if current_mtime > file_state.file_system_timestamp + 2000 {
                                    let message = format!(
                                        "Write blocked [edit_file_modified]: file '{}' changed after it was read. Use `Read` again before overwriting it.",
                                        resolved_path.display()
                                    );

                                    return Ok(ToolResult {
                                        llm_content: Some(message.clone()),
                                        return_display: Some(message.clone()),
                                        output: message.clone(),
                                        error: Some(crate::core::tools::tools::ToolError {
                                            error_type: ToolErrorType::EditFileModified.to_string(),
                                            message,
                                        }),
                                        data: None,
                                    });
                                }
                            }
                        }
                    }
                    existing_file_verified_for_overwrite = true;
                } else {
                    let message = format!(
                        "Write blocked [edit_file_not_read]: existing file '{}' must be read with `Read` before using `write_file`. REQUIRED NEXT STEP: call `Read` with file_path='{}' first, then retry `write_file`. Do NOT retry `write_file` without reading the file first.",
                        resolved_path.display(),
                        resolved_path.display()
                    );

                    return Ok(ToolResult {
                        llm_content: Some(message.clone()),
                        return_display: Some(message.clone()),
                        output: message.clone(),
                        error: Some(crate::core::tools::tools::ToolError {
                            error_type: ToolErrorType::EditFileNotRead.to_string(),
                            message,
                        }),
                        data: None,
                    });
                }
            }

            let corrected_result = get_corrected_file_content(
                &config,
                &resolved_path,
                &params.content,
                &tokio_util::sync::CancellationToken::new(),
            )
            .await?;

            if let Some(error) = &corrected_result.error {
                let error_msg = if let Some(code) = &error.code {
                    format!(
                        "Error checking existing file '{}': {} ({})",
                        resolved_path.display(),
                        error.message,
                        code
                    )
                } else {
                    format!("Error checking existing file: {}", error.message)
                };

                return Ok(ToolResult {
                    llm_content: Some(error_msg.clone()),
                    return_display: Some(error_msg.clone()),
                    output: error_msg.clone(),
                    error: Some(crate::core::tools::tools::ToolError {
                        error_type: ToolErrorType::FileWriteFailure.to_string(),
                        message: error_msg,
                    }),
                    data: None,
                });
            }

            let original_content = &corrected_result.original_content;
            let corrected_content = &corrected_result.corrected_content;
            let file_exists = corrected_result.file_exists;
            let original_line_ending = corrected_result.original_line_ending.clone();

            let is_new_file = !file_exists;

            // Restore CRLF line endings if the original file used them
            let content_to_write = if original_line_ending == super::edit::LineEnding::CRLF {
                corrected_content.replace('\n', "\r\n")
            } else {
                corrected_content.clone()
            };

            if let Some(message) = full_rewrite_guard_message(
                &resolved_path,
                original_content,
                corrected_content,
                is_new_file,
            ) {
                if !is_new_file
                    && existing_file_verified_for_overwrite
                    && full_rewrite_auto_recover_enabled()
                {
                    let current_content =
                        crate::core::utils::file_utils::read_file_with_encoding_async(
                            &resolved_path,
                        )
                        .await?;
                    let normalized_current = super::edit::normalize_line_endings(&current_content);
                    if normalized_current == *original_content {
                        write_content_atomically(&resolved_path, &content_to_write).await?;
                        let rel_path = resolved_path
                            .strip_prefix(std::env::current_dir().unwrap_or_default())
                            .unwrap_or(&resolved_path);
                        let success_message = format!(
                            "Updated {} (auto-recovered from full-file rewrite)",
                            rel_path.display()
                        );

                        return build_write_result(
                            &resolved_path,
                            original_content,
                            corrected_content,
                            &params,
                            success_message,
                            Some("guarded_exact_match_full_rewrite"),
                        );
                    }
                }

                return Ok(ToolResult {
                    llm_content: Some(message.clone()),
                    return_display: Some(message.clone()),
                    output: message.clone(),
                    error: Some(crate::core::tools::tools::ToolError {
                        error_type: ToolErrorType::FullFileRewriteBlocked.to_string(),
                        message,
                    }),
                    data: None,
                });
            }

            write_content_atomically(&resolved_path, &content_to_write).await?;

            let line_count = content_to_write.lines().count();
            let rel_path = resolved_path
                .strip_prefix(std::env::current_dir().unwrap_or_default())
                .unwrap_or(&resolved_path);
            let success_message = if is_new_file {
                format!("Wrote {} lines to {}", line_count, rel_path.display())
            } else {
                format!("Updated {} (+{} lines)", rel_path.display(), line_count)
            };

            build_write_result(
                &resolved_path,
                original_content,
                corrected_content,
                &params,
                success_message,
                None,
            )
        })
    }
}

pub struct WriteFileTool {
    config: Arc<crate::core::config::Config>,
    message_bus: Arc<MessageBus>,
    global_state: Arc<GlobalState>,
}

impl WriteFileTool {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            message_bus,
            global_state,
        }
    }

    pub fn name(&self) -> &str {
        "Write"
    }

    pub fn display_name(&self) -> &str {
        "WriteFile"
    }

    pub fn description(&self) -> &str {
        "Writes content to a specified file in the local filesystem. Prefer this for new files or small full rewrites. Existing files must be read first; large rewrites are blocked in favor of targeted edit tools."
    }

    pub fn kind(&self) -> Kind {
        Kind::Edit
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "properties": {
                "file_path": {
                    "description": "The path to the file to write to.",
                    "type": "string"
                },
                "content": {
                    "description": "The content to write to the file.",
                    "type": "string"
                }
            },
            "required": ["file_path", "content"],
            "type": "object"
        })
    }

    pub fn validate_tool_params(&self, params: &WriteFileToolParams) -> Result<(), String> {
        if params.file_path.is_empty() {
            return Err("Missing or empty \"file_path\"".to_string());
        }

        let resolved_path = self.config.target_dir().join(&params.file_path);

        if let Ok(metadata) = std::fs::metadata(&resolved_path) {
            if metadata.is_dir() {
                return Err(format!(
                    "Path is a directory, not a file: {}",
                    resolved_path.display()
                ));
            }
        }

        Ok(())
    }

    pub fn build(&self, params: WriteFileToolParams) -> Box<dyn ToolInvocation> {
        Box::new(WriteFileToolInvocation::new(
            self.config.clone(),
            params,
            self.message_bus.clone(),
            self.global_state.clone(),
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
        ))
    }
}

impl BaseDeclarativeTool for WriteFileTool {
    fn name(&self) -> &str {
        WriteFileTool::name(self)
    }

    fn display_name(&self) -> &str {
        WriteFileTool::display_name(self)
    }

    fn description(&self) -> &str {
        WriteFileTool::description(self)
    }

    fn kind(&self) -> Kind {
        WriteFileTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        WriteFileTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WriteFileToolParams = serde_json::from_value(params)?;
        self.validate_tool_params(&params)
            .map_err(|e| e.to_string())?;
        Ok(self.build(params))
    }
}

impl ModifiableDeclarativeTool<WriteFileToolParams> for WriteFileTool {
    fn get_modify_context(&self) -> ModifyContext<WriteFileToolParams> {
        let config_for_current = self.config.clone();
        let config_for_proposed = self.config.clone();

        ModifyContext {
            get_file_path: Box::new(|params: &WriteFileToolParams| params.file_path.clone()),
            get_current_content: Box::new(move |params: &WriteFileToolParams| {
                let config = config_for_current.clone();
                let file_path_str = params.file_path.clone();
                let content = params.content.clone();
                Box::pin(async move {
                    let file_path = config.target_dir().join(&file_path_str);
                    let result = get_corrected_file_content(
                        &config,
                        &file_path,
                        &content,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await?;
                    Ok(result.original_content)
                })
            }),
            get_proposed_content: Box::new(move |params: &WriteFileToolParams| {
                let config = config_for_proposed.clone();
                let file_path_str = params.file_path.clone();
                let content = params.content.clone();
                Box::pin(async move {
                    let file_path = config.target_dir().join(&file_path_str);
                    let result = get_corrected_file_content(
                        &config,
                        &file_path,
                        &content,
                        &tokio_util::sync::CancellationToken::new(),
                    )
                    .await?;
                    Ok(result.corrected_content)
                })
            }),
            create_updated_params: Box::new(
                |_old_content: String,
                 modified_proposed_content: String,
                 original_params: WriteFileToolParams| {
                    let content = original_params.content.clone();
                    WriteFileToolParams {
                        file_path: original_params.file_path.clone(),
                        content: modified_proposed_content,
                        modified_by_user: Some(true),
                        ai_proposed_content: Some(content),
                    }
                },
            ),
        }
    }
}
