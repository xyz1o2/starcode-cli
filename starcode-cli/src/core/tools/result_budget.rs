//! 工具结果预算控制（对标 Claude Code 的 applyToolResultBudget）
//!
//! 每个工具通过 `max_result_size_chars` 声明输出上限。
//! 超出上限的结果被截断，完整内容持久化到磁盘，
//! AI 只收到预览 + 文件路径。

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

// ── 默认限制 ──

/// 默认工具结果最大字符数
const DEFAULT_MAX_RESULT_SIZE_CHARS: usize = 30_000;

/// Bash 命令输出的最大字符数
const BASH_MAX_RESULT_SIZE_CHARS: usize = 30_000;

/// Skill 执行结果的最大字符数
const SKILL_MAX_RESULT_SIZE_CHARS: usize = 100_000;

/// FileRead 的结果不限制（避免 Read→file→Read 循环）
const FILE_READ_MAX_RESULT_SIZE_CHARS: usize = usize::MAX;

/// Grep/Glob 搜索结果的最大字符数
const SEARCH_MAX_RESULT_SIZE_CHARS: usize = 50_000;

/// 超限结果持久化目录
fn overflow_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".star")
        .join("tool-outputs")
}

/// 环境变量覆盖默认限制
fn env_max_result_size() -> Option<usize> {
    std::env::var("STAR_MAX_TOOL_RESULT_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
}

/// 获取工具的默认结果大小限制
pub fn default_max_result_size(tool_name: &str) -> usize {
    if let Some(env_val) = env_max_result_size() {
        return env_val;
    }

    match tool_name {
        "Bash" | "Shell" | "PowerShell" => BASH_MAX_RESULT_SIZE_CHARS,
        "Skill" | "SkillTool" => SKILL_MAX_RESULT_SIZE_CHARS,
        "Read" | "ReadFile" | "ReadMany" => FILE_READ_MAX_RESULT_SIZE_CHARS,
        "Grep" | "Glob" | "Search" | "Ripgrep" | "SemanticSearch" => SEARCH_MAX_RESULT_SIZE_CHARS,
        _ => DEFAULT_MAX_RESULT_SIZE_CHARS,
    }
}

/// 工具结果预算应用结果
#[derive(Debug)]
pub struct BudgetResult {
    /// 截断后的输出（发送给 LLM）
    pub truncated_output: String,
    /// 是否发生了截断
    pub was_truncated: bool,
    /// 持久化文件路径（如果截断且已保存）
    pub persisted_path: Option<PathBuf>,
    /// 原始输出字符数
    pub original_size: usize,
}

/// 应用工具结果预算（对标 Claude Code 的 applyToolResultBudget）
///
/// 如果输出超过限制：
/// 1. 将完整内容持久化到磁盘
/// 2. 返回截断后的预览 + 文件路径提示
///
/// # Arguments
/// - `tool_name`: 工具名称（用于获取默认限制）
/// - `output`: 工具输出内容
/// - `tool_max_chars`: 工具声明的最大字符数（覆盖默认值）
pub fn apply_tool_result_budget(
    tool_name: &str,
    output: &str,
    tool_max_chars: Option<usize>,
) -> BudgetResult {
    let max_chars = tool_max_chars.unwrap_or_else(|| default_max_result_size(tool_name));

    // 不限制的情况
    if max_chars == usize::MAX {
        return BudgetResult {
            truncated_output: output.to_string(),
            was_truncated: false,
            persisted_path: None,
            original_size: output.len(),
        };
    }

    // 未超出限制
    if output.len() <= max_chars {
        return BudgetResult {
            truncated_output: output.to_string(),
            was_truncated: false,
            persisted_path: None,
            original_size: output.len(),
        };
    }

    // 超出限制：持久化完整内容，返回截断预览
    let persisted_path = persist_tool_output(tool_name, output);

    // 截断到限制内，保留完整的 UTF-8 字符边界
    let safe_end = output
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(output.len());

    let mut truncated = String::with_capacity(max_chars + 200);
    truncated.push_str(&output[..safe_end]);

    // 添加截断提示
    truncated.push_str(&format!(
        "\n\n... [Output truncated: {} chars → {} chars. Full output saved to: {}. Set STAR_MAX_TOOL_RESULT_SIZE to adjust limit.]",
        output.len(),
        max_chars,
        persisted_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(failed to save)".to_string())
    ));

    BudgetResult {
        truncated_output: truncated,
        was_truncated: true,
        persisted_path,
        original_size: output.len(),
    }
}

/// 持久化工具输出到磁盘
fn persist_tool_output(tool_name: &str, output: &str) -> Option<PathBuf> {
    let dir = overflow_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ToolResultBudget] Failed to create overflow dir: {}",
            e
        ));
        return None;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let filename = format!("{}_{}.txt", tool_name.to_lowercase(), timestamp);
    let path = dir.join(filename);

    if let Err(e) = std::fs::write(&path, output) {
        crate::utils::logging::append_debug_log_line(&format!(
            "[ToolResultBudget] Failed to persist tool output: {}",
            e
        ));
        return None;
    }

    Some(path)
}

/// 清理过期的持久化工具输出（保留最近 N 个文件）
pub fn cleanup_old_outputs(max_files: usize) {
    let dir = overflow_dir();
    if !dir.exists() {
        return;
    }

    let mut files: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "txt").unwrap_or(false))
            .collect(),
        Err(_) => return,
    };

    if files.len() <= max_files {
        return;
    }

    // 按修改时间排序（最新的在前）
    files.sort_by(|a, b| {
        let a_time = a
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let b_time = b
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        b_time.cmp(&a_time)
    });

    // 删除多余的文件
    for file in files.iter().skip(max_files) {
        let _ = std::fs::remove_file(file.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_within_limit() {
        let result = apply_tool_result_budget("Bash", "short output", None);
        assert!(!result.was_truncated);
        assert_eq!(result.truncated_output, "short output");
    }

    #[test]
    fn test_budget_exceeds_limit() {
        let long_output = "x".repeat(50_000);
        let result = apply_tool_result_budget("Bash", &long_output, Some(1000));
        assert!(result.was_truncated);
        assert!(result.truncated_output.len() < long_output.len());
        assert!(result.truncated_output.contains("Output truncated"));
    }

    #[test]
    fn test_file_read_unlimited() {
        let long_output = "x".repeat(200_000);
        let result = apply_tool_result_budget("Read", &long_output, None);
        assert!(!result.was_truncated);
    }

    #[test]
    fn test_default_limits() {
        assert_eq!(default_max_result_size("Bash"), 30_000);
        assert_eq!(default_max_result_size("Read"), usize::MAX);
        assert_eq!(default_max_result_size("Grep"), 50_000);
        assert_eq!(default_max_result_size("UnknownTool"), 30_000);
    }
}
