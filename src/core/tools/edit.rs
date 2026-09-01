use crate::core::confirmation_bus::MessageBus;
use crate::core::state::{GlobalState, ReadFileState};
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::tools::{BaseDeclarativeTool, Kind, ToolError, ToolResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditToolParams {
    #[serde(rename = "file_path")]
    pub file_path: String,
    #[serde(rename = "old_string")]
    pub old_string: String,
    #[serde(rename = "new_string")]
    pub new_string: String,
    #[serde(rename = "expected_replacements")]
    pub expected_replacements: Option<usize>,
    pub instruction: Option<String>,
    #[serde(rename = "modified_by_user")]
    pub modified_by_user: Option<bool>,
    #[serde(rename = "ai_proposed_content")]
    pub ai_proposed_content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReplacementResult {
    pub new_content: String,
    pub occurrences: usize,
    pub final_old_string: String,
    pub final_new_string: String,
}

#[derive(Debug, Clone)]
pub struct CalculatedEdit {
    pub current_content: Option<String>,
    pub new_content: String,
    pub occurrences: usize,
    pub error: Option<EditError>,
    pub is_new_file: bool,
    pub original_line_ending: LineEnding,
}

#[derive(Debug, Clone)]
pub struct EditError {
    pub display: String,
    pub raw: String,
    pub error_type: ToolErrorType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineEnding {
    CRLF,
    LF,
}

pub fn apply_replacement(
    current_content: Option<&str>,
    old_string: &str,
    new_string: &str,
    is_new_file: bool,
) -> String {
    if is_new_file {
        return new_string.to_string();
    }

    let current = current_content.unwrap_or("");

    if old_string.is_empty() && !is_new_file {
        return current.to_string();
    }

    safe_literal_replace(current, old_string, new_string)
}

pub fn safe_literal_replace(content: &str, old: &str, new: &str) -> String {
    content.replace(old, new)
}

pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn restore_trailing_newline(original: &str, modified: &str) -> String {
    let had_trailing = original.ends_with('\n');

    if had_trailing && !modified.ends_with('\n') {
        format!("{}\n", modified)
    } else if !had_trailing && modified.ends_with('\n') {
        modified.trim_end_matches('\n').to_string()
    } else {
        modified.to_string()
    }
}

pub fn detect_line_ending(content: &str) -> LineEnding {
    if content.contains("\r\n") {
        LineEnding::CRLF
    } else {
        LineEnding::LF
    }
}

pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

pub fn escape_regex(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '[' | ']' | '|' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            _ => result.push(c),
        }
    }
    result
}

/// Strip line-number prefixes like "   45→" that models sometimes include
/// when copying from `Read` output. The `Read` tool returns lines
/// in `cat -n` format, and despite warnings, models occasionally include the
/// `NN→` prefix in `old_string`. This auto-strips it as a safety net.
pub fn strip_line_number_prefixes(s: &str) -> (String, bool) {
    let re = regex::Regex::new(r"(?m)^\s*\d+→\s*").unwrap();
    let stripped = re.replace_all(s, "").to_string();
    let was_stripped = stripped != s;
    (stripped, was_stripped)
}

pub fn calculate_exact_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    let normalized_code = normalize_line_endings(current_content);
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let exact_occurrences = normalized_code
        .split(&normalized_search)
        .count()
        .saturating_sub(1);

    if exact_occurrences > 0 {
        let modified_code =
            safe_literal_replace(&normalized_code, &normalized_search, &normalized_replace);
        let modified_code = restore_trailing_newline(current_content, &modified_code);

        Some(ReplacementResult {
            new_content: modified_code,
            occurrences: exact_occurrences,
            final_old_string: normalized_search,
            final_new_string: normalized_replace,
        })
    } else {
        None
    }
}

pub fn calculate_flexible_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    let normalized_code = normalize_line_endings(current_content);
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let source_lines: Vec<String> = normalized_code.lines().map(|l| l.to_string()).collect();
    let search_lines_stripped: Vec<String> = normalized_search
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    let replace_lines: Vec<&str> = normalized_replace.lines().collect();

    let mut flexible_occurrences = 0;
    let mut i = 0;

    while i
        <= source_lines
            .len()
            .saturating_sub(search_lines_stripped.len())
    {
        let window = &source_lines[i..i + search_lines_stripped.len()];
        let window_stripped: Vec<String> = window.iter().map(|l| l.trim().to_string()).collect();

        let is_match = window_stripped
            .iter()
            .enumerate()
            .all(|(idx, line)| line == &search_lines_stripped[idx]);

        if is_match {
            flexible_occurrences += 1;
            let first_line = window.get(0).map(|s| s.as_str()).unwrap_or("");
            let indent_len = first_line.len() - first_line.trim_start().len();
            let indentation = &first_line[..indent_len];

            let new_block_with_indent: Vec<String> = replace_lines
                .iter()
                .map(|line| format!("{}{}", indentation, line))
                .collect();

            let mut new_source_lines = source_lines.clone();
            new_source_lines.splice(i..i + search_lines_stripped.len(), new_block_with_indent);
            // i += replace_lines.len();
            // 用新结果继续匹配
            return Some(ReplacementResult {
                new_content: restore_trailing_newline(
                    current_content,
                    &new_source_lines.join("\n"),
                ),
                occurrences: flexible_occurrences,
                final_old_string: normalized_search,
                final_new_string: normalized_replace,
            });
        } else {
            i += 1;
        }
    }

    if flexible_occurrences > 0 {
        let modified_code = source_lines.join("\n");
        let modified_code = restore_trailing_newline(current_content, &modified_code);

        Some(ReplacementResult {
            new_content: modified_code,
            occurrences: flexible_occurrences,
            final_old_string: normalized_search,
            final_new_string: normalized_replace,
        })
    } else {
        None
    }
}

pub fn calculate_regex_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> Option<ReplacementResult> {
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    let delimiters = ['(', ')', ':', '[', ']', '{', '}', '>', '<', '='];

    let mut processed_string = normalized_search.clone();
    for delim in delimiters {
        processed_string = processed_string.replace(delim, &format!(" {} ", delim));
    }

    let tokens: Vec<&str> = processed_string.split_whitespace().collect();

    if tokens.is_empty() {
        return None;
    }

    let escaped_tokens: Vec<String> = tokens.iter().map(|t| escape_regex(t)).collect();
    let pattern = escaped_tokens.join(r"\s*");
    // (?m) enables multi-line mode so ^ matches start of each line, not just start of string
    let final_pattern = format!(r"(?m)^(\s*){}", pattern);

    let regex = regex::Regex::new(&final_pattern).ok()?;

    if let Some(captures) = regex.captures(current_content) {
        let indentation = captures.get(1).map(|m| m.as_str()).unwrap_or("");
        let new_lines: Vec<&str> = normalized_replace.lines().collect();
        let new_block_with_indent: String = new_lines
            .iter()
            .map(|line| format!("{}{}", indentation, line))
            .collect::<Vec<_>>()
            .join("\n");

        let modified_code = regex
            .replace(current_content, &new_block_with_indent)
            .to_string();
        let modified_code = restore_trailing_newline(current_content, &modified_code);

        Some(ReplacementResult {
            new_content: modified_code,
            occurrences: 1,
            final_old_string: normalized_search,
            final_new_string: normalized_replace,
        })
    } else {
        None
    }
}

pub fn calculate_replacement(
    current_content: &str,
    old_string: &str,
    new_string: &str,
) -> ReplacementResult {
    let normalized_search = normalize_line_endings(old_string);
    let normalized_replace = normalize_line_endings(new_string);

    if normalized_search.is_empty() {
        return ReplacementResult {
            new_content: current_content.to_string(),
            occurrences: 0,
            final_old_string: normalized_search,
            final_new_string: normalized_replace,
        };
    }

    // Try original old_string first.
    if let Some(result) = calculate_exact_replacement(current_content, old_string, new_string) {
        return result;
    }

    if let Some(result) = calculate_flexible_replacement(current_content, old_string, new_string) {
        return result;
    }

    if let Some(result) = calculate_regex_replacement(current_content, old_string, new_string) {
        return result;
    }

    // Auto-strip line-number prefixes (e.g. "   45→") that models sometimes
    // accidentally include when copying from read_file output.
    let (stripped_old, was_stripped) = strip_line_number_prefixes(old_string);
    if was_stripped {
        crate::utils::logging::append_debug_log_line(
            "[Edit] Auto-stripped line-number prefix from old_string",
        );

        if let Some(result) =
            calculate_exact_replacement(current_content, &stripped_old, new_string)
        {
            return result;
        }
        if let Some(result) =
            calculate_flexible_replacement(current_content, &stripped_old, new_string)
        {
            return result;
        }
        if let Some(result) =
            calculate_regex_replacement(current_content, &stripped_old, new_string)
        {
            return result;
        }
    }

    ReplacementResult {
        new_content: current_content.to_string(),
        occurrences: 0,
        final_old_string: normalized_search,
        final_new_string: normalized_replace,
    }
}

pub fn get_error_replace_result(
    params: &EditToolParams,
    occurrences: usize,
    expected_replacements: usize,
    final_old_string: &str,
    final_new_string: &str,
) -> Option<EditError> {
    if occurrences == 0 {
        // Enhanced diagnosis: try to find similar content in the file
        let diagnosis = diagnose_replace_failure(&params.file_path, &params.old_string);
        Some(EditError {
            display: "Failed to edit, could not find the string to replace.".to_string(),
            raw: format!(
                "Failed to edit, 0 occurrences found for old_string in {}. \
                 The string to replace was not found in the file (even after trying exact, flexible-indent, and regex-fuzzy matching). \
                 \n\n{}\n\n\
                 Next step: re-read the file with Read to get the current content, then retry with the exact text.",
                params.file_path, diagnosis
            ),
            error_type: ToolErrorType::EditNoOccurrenceFound,
        })
    } else if occurrences != expected_replacements {
        let occurrence_term = if expected_replacements == 1 {
            "occurrence"
        } else {
            "occurrences"
        };

        Some(EditError {
            display: format!(
                "Failed to edit, expected {} {} but found {}.",
                expected_replacements, occurrence_term, occurrences
            ),
            raw: format!(
                "Failed to edit, Expected {} {} but found {} for old_string in file: {}",
                expected_replacements, occurrence_term, occurrences, params.file_path
            ),
            error_type: ToolErrorType::EditExpectedOccurrenceMismatch,
        })
    } else if final_old_string == final_new_string {
        let detail = if params.old_string != params.new_string {
            " (they differ only in line endings: old_string uses CRLF, new_string uses LF)"
                .to_string()
        } else {
            String::new()
        };
        Some(EditError {
            display: format!(
                "No changes to apply: old_string and new_string are identical after normalization.{}",
                detail
            ),
            raw: format!(
                "No changes to apply. The old_string ({:?}) and new_string ({:?}) are identical \
                 after line-ending normalization in file: {}. \
                 This means the replacement would not change the file. \
                 Check that old_string and new_string have genuinely different content.",
                params.old_string, params.new_string, params.file_path
            ),
            error_type: ToolErrorType::EditNoChange,
        })
    } else {
        None
    }
}

/// Diagnose why a replace operation failed by analyzing the file content
fn diagnose_replace_failure(file_path: &str, old_string: &str) -> String {
    let mut diagnosis = String::new();

    // Try to read the file
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            diagnosis.push_str(&format!("Could not read file for diagnosis: {}", e));
            return diagnosis;
        }
    };

    // Check 1: Partial match (most common)
    let old_trimmed = old_string.trim();
    if old_trimmed.len() > 10 {
        // Try to find first and last lines of old_string
        let first_line = old_trimmed.lines().next().unwrap_or("").trim();
        let last_line = old_trimmed.lines().last().unwrap_or("").trim();

        if !first_line.is_empty() && content.contains(first_line) {
            diagnosis.push_str(&format!(
                "DIAGNOSIS: Found partial match for the first line ('{}'). \
                 The old_string might have extra whitespace or different indentation. \
                 Suggestion: Read to get exact content, then copy-paste the exact text.",
                first_line.chars().take(50).collect::<String>()
            ));
            return diagnosis;
        }
    }

    // Check 2: Case sensitivity
    let content_lower = content.to_lowercase();
    let old_lower = old_string.to_lowercase();
    if content_lower.contains(&old_lower) && !content.contains(old_string) {
        diagnosis.push_str(
            "DIAGNOSIS: Found case-insensitive match. The old_string has different case than the file content. \
             Suggestion: use the exact case from the file."
        );
        return diagnosis;
    }

    // Check 3: Whitespace differences
    let old_normalized: String = old_string.chars().filter(|c| !c.is_whitespace()).collect();
    let content_normalized: String = content.chars().filter(|c| !c.is_whitespace()).collect();
    if content_normalized.contains(&old_normalized) && !content.contains(old_string) {
        diagnosis.push_str(
            "DIAGNOSIS: Found match after ignoring whitespace. The old_string has different whitespace than the file content. \
             Suggestion: Read to get exact content, including spaces and tabs."
        );
        return diagnosis;
    }

    // Check 4: Line ending differences
    let old_lf = old_string.replace("\r\n", "\n");
    let content_lf = content.replace("\r\n", "\n");
    if content_lf.contains(&old_lf) && !content.contains(old_string) {
        diagnosis.push_str(
            "DIAGNOSIS: Found match after normalizing line endings. The file might use different line endings (CRLF vs LF). \
             Suggestion: the system should handle this automatically, but try reading the file again."
        );
        return diagnosis;
    }

    // Check 5: Substring match
    if old_trimmed.len() > 20 {
        let substr = &old_trimmed[..old_trimmed.len() / 2];
        if content.contains(substr) {
            diagnosis.push_str(&format!(
                "DIAGNOSIS: Found partial match for first half of old_string ('{}...'). \
                 The full string might have been modified or might not exist in the file. \
                 Suggestion: Read to verify the current content.",
                substr.chars().take(30).collect::<String>()
            ));
            return diagnosis;
        }
    }

    // Default diagnosis
    diagnosis.push_str(
        "DIAGNOSIS: No similar content found in the file. \
         Common causes: (1) typo in old_string, (2) file was already modified, \
         (3) wrong file path. Suggestion: Read to get current content.",
    );

    diagnosis
}

pub struct EditTool {
    pub config: Arc<crate::core::config::Config>,
    pub message_bus: Arc<MessageBus>,
    pub global_state: Arc<GlobalState>,
}

impl EditTool {
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
        "Edit"
    }

    pub fn display_name(&self) -> &str {
        "EditFile"
    }

    pub fn description(&self) -> &str {
        "Replaces a string in a file with a new string. This tool requires that the file has been read first to ensure you have the correct context."
    }

    pub fn kind(&self) -> Kind {
        Kind::Edit
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "expected_replacements": { "type": "number" }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }
}

pub struct EditToolInvocation {
    config: Arc<crate::core::config::Config>,
    params: EditToolParams,
    message_bus: Arc<MessageBus>,
    global_state: Arc<GlobalState>,
}

impl EditToolInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: EditToolParams,
        message_bus: Arc<MessageBus>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            message_bus,
            global_state,
        }
    }
}

impl crate::core::tools::tools::ToolInvocation for EditToolInvocation {
    fn get_description(&self) -> String {
        format!("Edit file: {}", self.params.file_path)
    }

    fn tool_locations(&self) -> Vec<crate::core::tools::tools::ToolLocation> {
        vec![crate::core::tools::tools::ToolLocation {
            path: std::path::PathBuf::from(&self.params.file_path),
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
                > + Send,
        >,
    > {
        let config = self.config.clone();
        let path_str = self.params.file_path.clone();
        Box::pin(async move {
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let path = std::path::Path::new(&path_str);
                    if !tf.is_path_trusted(path).unwrap_or(false) {
                        return Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                             confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                             title: "Untrusted Edit".to_string(),
                             prompt: format!("Security: Editing file in untrusted path {:?} is blocked. Do you want to proceed?", path),
                             on_confirm: std::sync::Arc::new(move |_outcome| {
                                 // Placeholder for trust logic
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
        let global_state = self.global_state.clone();

        Box::pin(async move {
            // Re-use logic from original EditTool::execute
            if config.folder_trust() {
                if let Some(tf) = config.trusted_folders() {
                    let path = std::path::Path::new(&params.file_path);
                    if !tf.is_path_trusted(path).unwrap_or(false) {
                        let msg = format!("Security: Path {:?} is not in a trusted folder.", path);
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: "SecurityError".to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                }
            }

            // Resolve path consistently with read_file (join with target_dir)
            let resolved_path = config.target_dir().join(&params.file_path);
            let abs_path = resolved_path
                .canonicalize()
                .unwrap_or_else(|_| resolved_path.clone())
                .to_string_lossy()
                .to_string();

            // Disable strict read check if STAR_DISABLE_READ_CHECK is true
            let strict_read_check = std::env::var("STAR_DISABLE_READ_CHECK")
                .map(|v| v.to_lowercase() != "true" && v != "1")
                .unwrap_or(true); // Default to true (strict check enabled)

            if strict_read_check {
                let read_state = global_state.read_file_state.read().await;
                if let Some(file_state) = read_state.get(&abs_path) {
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
                                    // Mtime changed — but the content may still be the same (e.g.
                                    // cargo check metadata writes, filesystem timestamp quirks,
                                    // touch, or assistant's own prior edits via bash/write_file).
                                    // Read current content and compare before blocking.
                                    let content_changed = match crate::core::utils::file_utils::read_file_with_encoding_async(&resolved_path).await {
                                        Ok(current_content) => current_content != file_state.content,
                                        Err(_) => true, // can't read — assume changed
                                    };
                                    if content_changed {
                                        let msg = format!("File '{}' has been modified since you last read it. Please read the file again to ensure you are editing the latest version.", params.file_path);
                                        return Ok(ToolResult {
                                            llm_content: Some(msg.clone()),
                                            return_display: Some(format!("Error: {}", msg)),
                                            output: msg.clone(),
                                            error: Some(ToolError {
                                                error_type: ToolErrorType::EditFileModified
                                                    .to_string(),
                                                message: msg,
                                            }),
                                            data: None,
                                        });
                                    }
                                    // Content unchanged — update the recorded timestamp to suppress
                                    // future false positives, then proceed with the edit.
                                    let mut state = global_state.read_file_state.write().await;
                                    if let Some(fs) = state.get_mut(&abs_path) {
                                        fs.file_system_timestamp = current_mtime;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Not in state
                    // Allow if file doesn't exist (creating new file)
                    if std::path::Path::new(&params.file_path).exists() {
                        let msg = format!(
                            "Edit blocked [edit_file_not_read]: file '{}' must be read with `Read` before using `replace`. \
                             REQUIRED NEXT STEP: call `Read` with file_path='{}' first, then retry. \
                             Do NOT retry without reading the file first.",
                            params.file_path, params.file_path
                        );
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: ToolErrorType::EditFileNotRead.to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                }
            }

            // Early check: if old_string and new_string are identical after line-ending normalization,
            // there's nothing to do. This catches LLM mistakes before unnecessary file I/O.
            {
                let normalized_old = normalize_line_endings(&params.old_string);
                let normalized_new = normalize_line_endings(&params.new_string);
                if normalized_old == normalized_new {
                    let msg = if params.old_string != params.new_string {
                        "No changes to apply: old_string and new_string differ only in line endings (CRLF vs LF). \
                         After normalizing \\r\\n→\\n they are identical. \
                         Ensure old_string and new_string have different content, not just different line endings.".to_string()
                    } else {
                        "No changes to apply: old_string and new_string are identical. \
                         You must provide different old_string (text to find) and new_string (replacement text).".to_string()
                    };
                    return Ok(ToolResult {
                        llm_content: Some(msg.clone()),
                        return_display: Some(msg.clone()),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: ToolErrorType::EditNoChange.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }
            }

            // File-history checkpoint: snapshot the file BEFORE we edit it.
            // track_edit is async (awaits IO), so it must run BEFORE
            // spawn_blocking captures `params`. Failures are best-effort —
            // they must never block the edit.
            {
                let msg_id = global_state.current_message_id().await;
                let edit_file_path = std::path::Path::new(&params.file_path).to_path_buf();
                if let Err(e) = crate::utils::checkpoint_manager::track_edit(
                    &edit_file_path,
                    msg_id,
                    Some("edit"),
                    None, // session_id: per-cwd fallback, matches /undo and /rewind
                )
                .await
                {
                    log::warn!(
                        "FileHistory: track_edit failed for {}: {}",
                        params.file_path,
                        e
                    );
                }
            }

            let result = tokio::task::spawn_blocking(move || {
                let expected_replacements = params.expected_replacements.unwrap_or(1);

                let mut current_content: Option<String> = None;
                let mut _file_exists = false;
                let mut original_line_ending = LineEnding::LF;

                match crate::core::utils::file_utils::read_file_with_encoding_io(
                    std::path::Path::new(&params.file_path),
                ) {
                    Ok(content) => {
                        original_line_ending = detect_line_ending(&content);
                        current_content = Some(normalize_line_endings(&content));
                        _file_exists = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        _file_exists = false;
                    }
                    Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                }

                let is_new_file = params.old_string.is_empty() && !_file_exists;

                if is_new_file {
                    let line_count = params.new_string.lines().count();
                    let msg = format!("Wrote {} lines to {}", line_count, params.file_path);
                    // Create parent directory if needed
                    if let Some(parent) = Path::new(&params.file_path).parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    }
                    // Atomic write: write to temp then rename
                    let tmp_path = format!("{}.star_tmp", &params.file_path);
                    std::fs::write(&tmp_path, &params.new_string)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                    std::fs::rename(&tmp_path, &params.file_path)
                        .or_else(|_| {
                            std::fs::copy(&tmp_path, &params.file_path)
                                .map(|_| ())
                                .and_then(|_| std::fs::remove_file(&tmp_path))
                        })
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

                    let line_count = params.new_string.lines().count();
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "Wrote {} lines to {}",
                            line_count, params.file_path
                        )),
                        return_display: Some(format!(
                            "Wrote {} lines to {}",
                            line_count, params.file_path
                        )),
                        output: msg.clone(),
                        error: None,
                        data: None,
                    });
                }

                if !_file_exists {
                    let msg = format!("File not found: {}", params.file_path);
                    return Ok(ToolResult {
                        llm_content: Some(format!("File not found: {}", params.file_path)),
                        return_display: Some(
                            "Error: File not found. Cannot apply edit.".to_string(),
                        ),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: ToolErrorType::FileNotFound.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }

                let current = current_content.as_ref().unwrap();

                if params.old_string.is_empty() {
                    let msg = format!("File already exists, cannot create: {}", params.file_path);
                    return Ok(ToolResult {
                        llm_content: Some(format!(
                            "File already exists, cannot create: {}",
                            params.file_path
                        )),
                        return_display: Some("Error: File already exists.".to_string()),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: ToolErrorType::AttemptToCreateExistingFile.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }

                let replacement_result =
                    calculate_replacement(current, &params.old_string, &params.new_string);

                if let Some(error) = get_error_replace_result(
                    &params,
                    replacement_result.occurrences,
                    expected_replacements,
                    &replacement_result.final_old_string,
                    &replacement_result.final_new_string,
                ) {
                    let msg = error.raw.to_string();
                    return Ok(ToolResult {
                        llm_content: Some(msg.clone()),
                        return_display: Some(format!("Error: {}", error.display)),
                        output: msg.clone(),
                        error: Some(ToolError {
                            error_type: error.error_type.to_string(),
                            message: msg,
                        }),
                        data: None,
                    });
                }

                let mut final_content = replacement_result.new_content.clone();

                if !is_new_file && original_line_ending == LineEnding::CRLF {
                    final_content = final_content.replace('\n', "\r\n");
                }

                if let Some(parent) = Path::new(&params.file_path).parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                }

                // Atomic write: write to temp then rename
                let tmp_path = format!("{}.star_tmp", &params.file_path);
                std::fs::write(&tmp_path, &final_content)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                std::fs::rename(&tmp_path, &params.file_path)
                    .or_else(|_| {
                        std::fs::copy(&tmp_path, &params.file_path)
                            .map(|_| ())
                            .and_then(|_| std::fs::remove_file(&tmp_path))
                    })
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

                // Generate Diff
                let diff = TextDiff::from_lines(current, &final_content);
                let diff_output = format!(
                    "{}",
                    diff.unified_diff()
                        .header(&params.file_path, &params.file_path)
                );

                let added = params.new_string.lines().count();
                let removed = params.old_string.lines().count();
                let msg = if replacement_result.occurrences == 1 {
                    format!(
                        "Updated {} (+{} -{})",
                        params.file_path,
                        added.saturating_sub(removed),
                        removed.saturating_sub(added)
                    )
                } else {
                    format!(
                        "Updated {} ({} replacements, +{} -{})",
                        params.file_path,
                        replacement_result.occurrences,
                        added.saturating_sub(removed),
                        removed.saturating_sub(added)
                    )
                };
                Ok(ToolResult {
                    llm_content: Some(msg.clone()),
                    return_display: Some(format!(
                        "Modified {} ({} replacements)",
                        params.file_path, replacement_result.occurrences
                    )),
                    output: msg,
                    error: None,
                    data: Some(serde_json::json!({
                        "diff": diff_output
                    })),
                })
            })
            .await;

            match result {
                Ok(inner_result) => {
                    // Update ReadFileState after successful edit so subsequent edits don't see stale mtime
                    if inner_result.is_ok()
                        && inner_result
                            .as_ref()
                            .map(|r| r.error.is_none())
                            .unwrap_or(false)
                    {
                        let file_system_timestamp = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        let timestamp = file_system_timestamp;

                        // Read current content for the state update
                        if let Ok(content) =
                            crate::core::utils::file_utils::read_file_with_encoding_async(
                                &resolved_path,
                            )
                            .await
                        {
                            let mut state = global_state.read_file_state.write().await;
                            state.insert(
                                abs_path.clone(),
                                ReadFileState {
                                    content,
                                    timestamp,
                                    file_system_timestamp,
                                },
                            );
                        }
                    }

                    inner_result.map_err(|e| e as Box<dyn std::error::Error>)
                }
                Err(e) => Err(Box::new(e)),
            }
        })
    }
}

impl BaseDeclarativeTool for EditTool {
    fn name(&self) -> &str {
        EditTool::name(self)
    }

    fn display_name(&self) -> &str {
        EditTool::display_name(self)
    }

    fn description(&self) -> &str {
        EditTool::description(self)
    }

    fn kind(&self) -> Kind {
        EditTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        EditTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<
        Box<dyn crate::core::tools::tools::ToolInvocation>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let edit_params: EditToolParams = serde_json::from_value(params)?;
        Ok(Box::new(EditToolInvocation::new(
            self.config.clone(),
            edit_params,
            self.message_bus.clone(),
            self.global_state.clone(),
        )))
    }
}
