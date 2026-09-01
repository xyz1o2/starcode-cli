// ============================================================================
// LLM 处理策略
// ============================================================================
//
// 当所有自动策略都失败时，使用 LLM 分析原因并处理
//
// 工作流程：
// 1. 构建详细的上下文提示（文件内容、用户意图）
// 2. 要求 LLM 分析为什么匹配失败
// 3. LLM 返回修正后的 old_string 和 new_string
// 4. 重新尝试精确匹配
//
// 适用场景：
// - 用户提供的代码片段不完整
// - 语法错误
// - 其他无法自动处理的情况

use super::{EditContext, EditResult, EditStrategy};
use crate::llm::client::StarClient;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub struct LlmFixStrategy {
    client: StarClient,
}

impl LlmFixStrategy {
    pub fn new(client: StarClient) -> Self {
        Self { client }
    }

    fn truncate_chars(input: &str, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }

        let mut out = String::new();
        for (idx, ch) in input.chars().enumerate() {
            if idx >= max_chars {
                out.push_str("\n... [truncated]");
                break;
            }
            out.push(ch);
        }
        out
    }

    /// 构建 LLM 处理提示
    fn build_fix_prompt(context: &EditContext) -> String {
        let max_file_chars = std::env::var("STAR_LLM_FIX_MAX_FILE_CHARS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(8000);
        let max_old_chars = std::env::var("STAR_LLM_FIX_MAX_OLD_CHARS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2000);
        let max_new_chars = std::env::var("STAR_LLM_FIX_MAX_NEW_CHARS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2000);

        let file_preview = Self::truncate_chars(&context.content, max_file_chars);
        let old_preview = Self::truncate_chars(&context.old_string, max_old_chars);
        let new_preview = Self::truncate_chars(&context.new_string, max_new_chars);

        format!(
            r#"You are a code editing assistant. The user wants to replace code in a file, but automatic matching failed.

File path: {}
File content:
```
{}
```

Code to replace:
Old code:
```
{}
```

New code:
```
{}
```

Analyze why automatic matching failed and provide corrected old_string and new_string.

Requirements:
1. Ensure corrected_old_string can be exactly matched in the file content
2. Preserve the user's intent (replacement logic)
3. Return only JSON format, no other explanation

Return format:
{{
    "corrected_old_string": "corrected old code that can match",
    "corrected_new_string": "corrected new code",
    "reason": "why original match failed"
}}"#,
            context.file_path, file_preview, old_preview, new_preview
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LlmFixResponse {
    corrected_old_string: String,
    corrected_new_string: String,
    reason: String,
}

#[async_trait]
impl EditStrategy for LlmFixStrategy {
    fn name(&self) -> &'static str {
        "llm_fix"
    }

    fn priority(&self) -> u32 {
        100 // 最低优先级（最后尝试）
    }

    fn is_enabled(&self) -> bool {
        // 可以通过环境变量控制
        std::env::var("STAR_ENABLE_LLM_FIX")
            .map(|v| {
                let v = v.to_lowercase();
                v == "1" || v == "true" || v == "on"
            })
            .unwrap_or(false) // 默认关闭（按需开启）
    }

    async fn try_edit(
        &self,
        context: &EditContext,
    ) -> Result<Option<EditResult>, Box<dyn std::error::Error + Send + Sync>> {
        // 1. 检查是否启用
        if !self.is_enabled() {
            return Ok(None);
        }

        crate::utils::logging::append_debug_log_line("[LLM Fix] trying LLM fix strategy...");

        // 2. 构建提示
        let prompt = Self::build_fix_prompt(context);

        // 3. 调用 LLM
        let timeout_secs = std::env::var("STAR_LLM_FIX_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(20);
        let response_text = match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            self.client.chat_completion_simple(&prompt),
        )
        .await
        {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[LLM Fix] LLM call failed: {}",
                    e
                ));
                return Ok(None);
            }
            Err(_) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[LLM Fix] LLM call timed out: {}s",
                    timeout_secs
                ));
                return Ok(None);
            }
        };

        // 4. 解析 JSON (处理可能的 Markdown 代码块)
        let json_str = if let Some(start) = response_text.find("```json") {
            let rest = &response_text[start + 7..];
            if let Some(end) = rest.find("```") {
                &rest[..end]
            } else {
                rest
            }
        } else if let Some(start) = response_text.find("```") {
            let rest = &response_text[start + 3..];
            if let Some(end) = rest.find("```") {
                &rest[..end]
            } else {
                rest
            }
        } else {
            &response_text
        };

        let json_str = json_str.trim();

        let fix_response: LlmFixResponse = match serde_json::from_str(json_str) {
            Ok(res) => res,
            Err(e) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[LLM Fix] JSON parse failed: {} | raw: {}",
                    e, response_text
                ));
                return Ok(None);
            }
        };

        crate::utils::logging::append_debug_log_line(&format!(
            "[LLM Fix] LLM suggestion: {}",
            fix_response.reason
        ));

        // 5. 尝试使用修正后的代码进行替换
        // 检查修正后的 old_string 是否存在
        if context.content.contains(&fix_response.corrected_old_string) {
            // 只替换第一个匹配项，避免误伤
            let new_content = context.content.replacen(
                &fix_response.corrected_old_string,
                &fix_response.corrected_new_string,
                1,
            );

            Ok(Some(EditResult {
                success: true,
                new_content,
                occurrences: 1,
                strategy: "llm_fix".to_string(),
                details: None,
            }))
        } else {
            crate::utils::logging::append_debug_log_line(
                "[LLM Fix] corrected code still not found in file",
            );
            Ok(None)
        }
    }
}

 