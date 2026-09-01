// ============================================================================
// 正则模糊匹配策略
// ============================================================================
//
// 将代码块转换为弹性正则表达式进行模糊匹配
//
// 原理：
// 1. 将代码分词（按空白和特殊符号）
// 2. 为每个 token 构建正则模式
// 3. token 之间允许任意空白（\s*）
// 4. 使用正则查找并替换
//
// 适用场景：
// - 代码格式化工具修改了空白
// - 换行位置不同
// - 注释位置变化

use super::{EditContext, EditResult, EditStrategy};
use async_trait::async_trait;
use regex::Regex;

pub struct RegexFuzzyStrategy;

impl RegexFuzzyStrategy {
    /// 转义正则表达式特殊字符
    fn escape_regex(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('.', "\\.")
            .replace('+', "\\+")
            .replace('*', "\\*")
            .replace('?', "\\?")
            .replace('^', "\\^")
            .replace('$', "\\$")
            .replace('(', "\\(")
            .replace(')', "\\)")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace('{', "\\{")
            .replace('}', "\\}")
            .replace('|', "\\|")
    }

    /// 将代码字符串转换为弹性正则表达式
    fn build_fuzzy_regex(code: &str) -> Result<Regex, regex::Error> {
        // 分隔符列表
        const DELIMITERS: &[char] = &['(', ')', ':', '[', ']', '{', '}', '>', '<', '='];

        // 按分隔符和空白分词
        let mut processed = code.to_string();
        for delim in DELIMITERS {
            processed = processed.replace(*delim, &format!(" {} ", delim));
        }

        // 分词并过滤空白
        let tokens: Vec<&str> = processed.split_whitespace().collect();

        if tokens.is_empty() {
            return Regex::new(""); // 空模式
        }

        // 构建正则表达式：token1\s*token2\s*token3...
        let pattern = tokens
            .iter()
            .map(|token| Self::escape_regex(token))
            .collect::<Vec<_>>()
            .join(r"\s*");

        Regex::new(&pattern)
    }
}

#[async_trait]
impl EditStrategy for RegexFuzzyStrategy {
    fn name(&self) -> &'static str {
        "regex_fuzzy"
    }

    fn priority(&self) -> u32 {
        20 // 较低优先级
    }

    fn is_enabled(&self) -> bool {
        // 可以通过环境变量控制
        std::env::var("STAR_ENABLE_REGEX_FUZZY")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(true) // 默认启用
    }

    async fn try_edit(
        &self,
        context: &EditContext,
    ) -> Result<Option<EditResult>, Box<dyn std::error::Error + Send + Sync>> {
        // Normalize CRLF -> LF for consistent matching
        let normalized_content = crate::core::tools::edit::normalize_line_endings(&context.content);
        let normalized_old = crate::core::tools::edit::normalize_line_endings(&context.old_string);
        let normalized_new = crate::core::tools::edit::normalize_line_endings(&context.new_string);

        // 构建弹性正则表达式
        let regex = match Self::build_fuzzy_regex(&normalized_old) {
            Ok(r) => r,
            Err(e) => {
                crate::utils::logging::append_debug_log_line(&format!(
                    "[RegexFuzzy] failed to build regex: {}",
                    e
                ));
                return Ok(None);
            }
        };

        // 查找所有匹配
        let matches: Vec<_> = regex.find_iter(&normalized_content).collect();

        if matches.is_empty() {
            return Ok(None);
        }

        // 执行替换
        let new_content = regex
            .replace_all(&normalized_content, &normalized_new)
            .to_string();

        Ok(Some(
            EditResult::success(new_content, matches.len(), self.name()).with_details(format!(
                "regex fuzzy match replaced {} occurrences",
                matches.len()
            )),
        ))
    }
}
