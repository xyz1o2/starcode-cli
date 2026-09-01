// ============================================================================
// 精确匹配策略
// ============================================================================
//
// 最简单、最快的策略：直接进行字符串替换
//
// 优点：
// - 速度最快
// - 无歧义
// - 适用于大多数场景
//
// 缺点：
// - 对空白字符敏感
// - 无法处理缩进差异

use super::{EditContext, EditResult, EditStrategy};
use async_trait::async_trait;

pub struct ExactMatchStrategy;

/// Normalize CRLF -> LF for matching, preserving knowledge of whether original had CRLF.
fn normalize_crlf(s: &str) -> (String, bool) {
    let had_crlf = s.contains("\r\n");
    (s.replace("\r\n", "\n"), had_crlf)
}

#[async_trait]
impl EditStrategy for ExactMatchStrategy {
    fn name(&self) -> &'static str {
        "exact_match"
    }

    fn priority(&self) -> u32 {
        0 // 最高优先级
    }

    async fn try_edit(
        &self,
        context: &EditContext,
    ) -> Result<Option<EditResult>, Box<dyn std::error::Error + Send + Sync>> {
        if context.old_string.is_empty() {
            return Ok(None);
        }

        // Normalize CRLF->LF for matching (same as the old `replace` tool).
        let (content_norm, had_crlf) = normalize_crlf(&context.content);
        let (old_norm, _) = normalize_crlf(&context.old_string);

        let content = content_norm.as_str();
        let old = old_norm.as_str();

        // Strategy 1: Try line-boundary match first (strict)
        let mut starts: Vec<usize> = Vec::new();
        for (idx, _) in content.match_indices(old) {
            let prev_ok = idx == 0 || content.as_bytes().get(idx.wrapping_sub(1)) == Some(&b'\n');
            let end = idx + old.len();
            let next_ok = end == content.len() || content.as_bytes().get(end) == Some(&b'\n');
            if prev_ok && next_ok {
                starts.push(idx);
            }
        }

        // Strategy 2: If line-boundary match failed, try simple substring match
        if starts.is_empty() {
            if let Some(idx) = content.find(old) {
                starts.push(idx);
            }
        }

        if starts.is_empty() {
            return Ok(None);
        }

        let mut new_content = String::new();
        let mut last = 0usize;
        for start in &starts {
            new_content.push_str(&content[last..*start]);
            new_content.push_str(&context.new_string);
            last = *start + old.len();
        }
        new_content.push_str(&content[last..]);
        let occurrences = starts.len();

        // Restore CRLF if the original file used it.
        let new_content = if had_crlf {
            new_content.replace('\n', "\r\n")
        } else {
            new_content
        };

        Ok(Some(
            EditResult::success(new_content, occurrences, self.name()).with_details(format!(
                "Exact match succeeded, {} replacements",
                occurrences
            )),
        ))
    }
}
