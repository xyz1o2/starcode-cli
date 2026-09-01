/// JIT Context Loader
/// 职责：将命令运行时输出裁剪成 LLM 上下文片段，供后续推理使用。
/// 不在此处编写任何硬编码的错误建议——那是 LLM 的工作。
pub struct JitContextLoader;

/// How many characters of raw output to keep.  Long build logs are truncated
/// to avoid blowing the context window.
const MAX_OUTPUT_CHARS: usize = 4096;

impl JitContextLoader {
    pub fn new() -> Self {
        Self
    }

    /// Wrap `output` as a JIT context block.
    ///
    /// Returns `None` when the output is blank (nothing useful to attach).
    /// The caller is responsible for prepending the returned string to the
    /// prompt so the LLM can reason over the actual error.
    pub fn analyze_output(&self, output: &str) -> Option<String> {
        let trimmed = output.trim();
        if trimmed.is_empty() {
            return None;
        }

        let body = if trimmed.len() > MAX_OUTPUT_CHARS {
            // Keep the tail — that is usually where errors appear.
            let start = trimmed.len() - MAX_OUTPUT_CHARS;
            // Align to a char boundary.
            let start = trimmed
                .char_indices()
                .map(|(i, _)| i)
                .filter(|&i| i >= start)
                .next()
                .unwrap_or(start);
            format!("[... truncated ...]\n{}", &trimmed[start..])
        } else {
            trimmed.to_string()
        };

        Some(format!("## Command Output\n\n```\n{}\n```", body))
    }
}
