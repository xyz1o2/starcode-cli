// ============================================================================
// 弹性缩进策略
// ============================================================================
//
// 忽略缩进和空白差异的匹配策略
//
// 原理：
// 1. 将搜索字符串和文件内容都去除缩进（trim）
// 2. 使用滑动窗口逐行匹配
// 3. 匹配成功后，保留原文件的缩进风格
//
// 适用场景：
// - 用户复制代码时丢失了缩进
// - 代码缩进风格不一致（2空格 vs 4空格）
// - Tab 与空格混用

use super::{EditContext, EditResult, EditStrategy};
use async_trait::async_trait;

pub struct FlexibleIndentStrategy;

impl FlexibleIndentStrategy {
    /// 提取行首的缩进
    fn extract_indentation(line: &str) -> &str {
        let trimmed_start = line.len() - line.trim_start().len();
        &line[..trimmed_start]
    }

    /// 对多行字符串的每一行应用缩进
    fn apply_indentation(content: &str, indentation: &str) -> String {
        content
            .lines()
            .map(|line| {
                if line.trim().is_empty() {
                    line.to_string() // 空行保持原样
                } else {
                    format!("{}{}", indentation, line.trim())
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[async_trait]
impl EditStrategy for FlexibleIndentStrategy {
    fn name(&self) -> &'static str {
        "flexible_indent"
    }

    fn priority(&self) -> u32 {
        10 // 次高优先级
    }

    async fn try_edit(
        &self,
        context: &EditContext,
    ) -> Result<Option<EditResult>, Box<dyn std::error::Error + Send + Sync>> {
        // Normalize CRLF -> LF for consistent matching
        let normalized_content = crate::core::tools::edit::normalize_line_endings(&context.content);
        let normalized_old = crate::core::tools::edit::normalize_line_endings(&context.old_string);
        let normalized_new = crate::core::tools::edit::normalize_line_endings(&context.new_string);

        // Strategy 1: Try line-by-line sliding window match (original behavior)
        let search_lines: Vec<String> = normalized_old
            .lines()
            .map(|line| line.trim().to_string())
            .collect();

        if search_lines.is_empty() {
            return Ok(None);
        }

        let mut result_lines: Vec<String> =
            normalized_content.lines().map(|s| s.to_string()).collect();

        let mut matches_found = 0;
        let mut i = 0;

        while i <= result_lines.len().saturating_sub(search_lines.len()) {
            let window_trimmed: Vec<String> = result_lines[i..i + search_lines.len()]
                .iter()
                .map(|line| line.trim().to_string())
                .collect();

            let is_match = window_trimmed
                .iter()
                .zip(search_lines.iter())
                .all(|(a, b)| a == b);

            if is_match {
                matches_found += 1;

                let original_indentation = Self::extract_indentation(&result_lines[i]);

                let indented_new_content =
                    Self::apply_indentation(&normalized_new, original_indentation);

                let mut new_result = Vec::new();
                new_result.extend_from_slice(&result_lines[..i]);
                new_result.extend(indented_new_content.lines().map(|s| s.to_string()));
                new_result.extend_from_slice(&result_lines[i + search_lines.len()..]);

                result_lines = new_result;

                i += indented_new_content.lines().count();
            } else {
                i += 1;
            }
        }

        if matches_found > 0 {
            let new_content = result_lines.join("\n");
            return Ok(Some(
                EditResult::success(new_content, matches_found, self.name()).with_details(format!(
                    "flexible indent match replaced {} occurrences (preserved original indentation)",
                    matches_found
                )),
            ));
        }

        // Strategy 2: If line-by-line match failed, try trimmed substring match
        // This handles cases where old_string spans partial lines
        let trimmed_content: String = normalized_content
            .lines()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join("\n");
        let trimmed_old: String = normalized_old
            .lines()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join("\n");

        if !trimmed_old.is_empty() {
            if let Some(idx) = trimmed_content.find(&trimmed_old) {
                // Find the corresponding position in the original content
                let mut original_pos = 0;
                let mut trimmed_pos = 0;
                for line in normalized_content.lines() {
                    let trimmed_line = line.trim();
                    if trimmed_pos + trimmed_line.len() > idx {
                        original_pos += idx - trimmed_pos;
                        break;
                    }
                    trimmed_pos += trimmed_line.len() + 1; // +1 for newline
                    original_pos += line.len() + 1;
                }

                let mut new_content = String::new();
                new_content.push_str(&normalized_content[..original_pos]);
                new_content.push_str(&normalized_new);
                let end_pos = original_pos + normalized_old.len();
                if end_pos < normalized_content.len() {
                    new_content.push_str(&normalized_content[end_pos..]);
                }

                return Ok(Some(
                    EditResult::success(new_content, 1, self.name()).with_details(
                        "flexible indent match replaced 1 occurrence (trimmed substring match)"
                            .to_string(),
                    ),
                ));
            }
        }

        Ok(None)
    }
}
