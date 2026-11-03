//! 把自然语言需求改写成**代码侧**的检索词。
//!
//! 为什么必须改写：`CodebaseSearch` 是**词法**引擎，索引里全是英文标识符、
//! 路径和英文注释。中文需求（「然后 后停 停来」）整句塞进 `query`，一个
//! token 都命中不了 —— 工具参数是"找代码用的检索词"，不是"复述用户需求"。
//!
//! 行业做法：Sourcegraph Cody 的 context engine 专门做 query rewriting，
//! 明确说是为了处理"长句与外语提问"；Cursor/Windsurf 则把这类工作放进检索
//! 子代理。这里取最小可行版本：**一次极小的 LLM 调用**，把需求映射成几个
//! 可能的英文标识符/术语；超时或失败即回退现有规则压缩，绝不拖住首轮。

use crate::llm::client::StarClient;
use std::time::Duration;

/// 改写超时。超时即回退规则压缩。
const REWRITE_TIMEOUT: Duration = Duration::from_secs(8);

/// 改写结果最多保留几个检索词。
const MAX_TERMS: usize = 6;

/// 需求里已经出现"代码味"的词时不必改写 —— 模型自己知道该搜什么。
fn has_code_like_token(input: &str) -> bool {
    input.split_whitespace().any(|raw| {
        let token = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':');
        token.chars().count() > 2
            && (token.contains('_')
                || token.contains("::")
                || token.contains('/')
                || token.chars().any(|c| c.is_uppercase()))
    })
}

/// 输入是否值得做一次 LLM 改写。
///
/// 判据：**含 CJK 且没有代码味 token**。纯英文提问由规则压缩处理（保留标识符、
/// 去掉虚词）已经够用；中文提问则必须翻译成英文标识符才有意义。
pub(crate) fn needs_rewrite(user_input: &str) -> bool {
    let has_cjk = user_input
        .chars()
        .any(|c| crate::core::context::search_engine::is_cjk_char(c));
    has_cjk && !has_code_like_token(user_input)
}

/// 把需求改写成检索词；返回 `None` 表示"没改成"（调用方回退规则压缩）。
pub(crate) async fn rewrite_search_query(client: &StarClient, user_input: &str) -> Option<String> {
    if !needs_rewrite(user_input) {
        return None;
    }

    let system = crate::types::StarMessage::system(
        "You translate a developer's request into code-search terms. Reply with ONLY a \
         space-separated list of at most 6 terms that would literally appear in source \
         code: identifiers (snake_case or CamelCase), type names, or technical nouns \
         in English. If you cannot suggest any plausible code term, reply with the single \
         word NONE. No explanation, no punctuation, no quotes.",
    );
    let user = crate::types::StarMessage::user(format!("Request: {user_input}\n\nTerms:"));

    let outcome = tokio::time::timeout(
        REWRITE_TIMEOUT,
        client.chat(vec![system, user], None, None, None),
    )
    .await;

    let text = match outcome {
        Ok(Ok(response)) => response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .unwrap_or("")
            .to_string(),
        Ok(Err(err)) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ACE_REWRITE] query rewrite failed: {err}"
            ));
            return None;
        }
        Err(_) => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ACE_REWRITE] query rewrite timed out after {}ms",
                REWRITE_TIMEOUT.as_millis()
            ));
            return None;
        }
    };

    let terms = parse_rewritten_terms(&text)?;
    crate::utils::logging::append_debug_log_line(&format!(
        "[ACE_REWRITE] query rewritten: {:?} -> {:?}",
        user_input, terms
    ));
    Some(terms.join(" "))
}

/// 英文虚词 —— 模型偶尔会回散文（"Sure! here you go: ..."）。
///
/// 这些词进了检索词表纯属噪声：它们在任何代码里都高频命中，等于把排序信号
/// 稀释掉。注意别收"search"/"index"/"cache"这类**领域词** —— 它们正是我们要的
/// 检索词。
const EN_PROSE_STOPWORDS: &[&str] = &[
    "i",
    "me",
    "my",
    "we",
    "our",
    "us",
    "you",
    "your",
    "he",
    "she",
    "it",
    "its",
    "they",
    "them",
    "their",
    "this",
    "that",
    "these",
    "those",
    "there",
    "here",
    "and",
    "or",
    "but",
    "not",
    "nor",
    "so",
    "if",
    "then",
    "else",
    "when",
    "while",
    "for",
    "from",
    "with",
    "without",
    "into",
    "onto",
    "upon",
    "over",
    "under",
    "above",
    "below",
    "between",
    "among",
    "of",
    "in",
    "on",
    "at",
    "by",
    "to",
    "as",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "do",
    "does",
    "did",
    "done",
    "have",
    "has",
    "had",
    "can",
    "cannot",
    "could",
    "shall",
    "should",
    "will",
    "would",
    "may",
    "might",
    "must",
    "please",
    "help",
    "sure",
    "okay",
    "yes",
    "no",
    "what",
    "which",
    "who",
    "whom",
    "whose",
    "where",
    "why",
    "how",
    "all",
    "any",
    "some",
    "each",
    "more",
    "most",
    "less",
    "least",
    "very",
    "just",
    "also",
    "only",
    "other",
    "others",
    "same",
    "such",
    "own",
    "too",
    "once",
    "twice",
    "use",
    "used",
    "using",
    "want",
    "wants",
    "need",
    "needs",
    "show",
    "shows",
    "tell",
    "tells",
    "give",
    "gives",
    "get",
    "gets",
    "make",
    "makes",
    "take",
    "takes",
    "thing",
    "things",
    "stuff",
    "code",
    "codes",
    "file",
    "files",
    "repo",
    "repos",
    "repository",
    "project",
    "projects",
    "codebase",
    "sorry",
    "unable",
    "list",
    "output",
    "inputs",
    "input",
    "the",
    "an",
    "am",
    "yet",
    "per",
    "via",
];

/// "元回答"黑名单 —— 模型有时不是给词，而是**回答"没有"**。
///
/// 现场事故：改写调用回了一句 `None`（意思是"我没想到英文标识符"），解析器
/// 把它当成合法检索词，于是拿 `None` 去搜纯英文代码库，命中一片 `none`。
/// 这些词一个都不是检索词，必须在解析层拒掉。
const META_ANSWER_TOKENS: &[&str] = &[
    "none",
    "null",
    "nil",
    "nothing",
    "nonefound",
    "notfound",
    "no",
    "n/a",
    "na",
    "unknown",
    "unspecified",
    "undefined",
    "empty",
    "tbd",
    "todo",
    "cannot",
    "unable",
    "notapplicable",
];

/// 解析模型回传的检索词。
///
/// 模型爱加编号、引号、解释文字 —— 这里只留"像标识符"的词：
/// 含字母、长度 ≥3、只含标识符字符（字母数字 `_ - . : #`），且不是英文虚词
/// 或"元回答"。全部被拒 → 返回 `None` → 调用方回退规则压缩。
pub(crate) fn parse_rewritten_terms(text: &str) -> Option<Vec<String>> {
    let mut terms: Vec<String> = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // 去掉 "1." "2)" "- " 这类前缀
        let line = line.trim_start_matches(|c: char| c == '-' || c == '*' || c == '•');
        let line = line.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')');
        let line = line.trim_start_matches(['"', '\'', '`', ':', ' ']);

        for raw in line.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
            // 只保留字母数字与下划线：冒号/点/短横线只允许出现在**内部**
            // （`std::io`、`foo-bar`），出现在两端就是标点（"go:" → "go"）。
            let token = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if token.is_empty() || token.chars().count() < 3 {
                continue;
            }
            if !token.chars().any(|c| c.is_ascii_alphabetic()) {
                continue;
            }
            let lower = token.to_lowercase();
            if EN_PROSE_STOPWORDS.contains(&lower.as_str()) {
                continue;
            }
            if META_ANSWER_TOKENS.contains(&lower.as_str()) {
                continue;
            }
            if terms
                .iter()
                .any(|existing| existing.to_lowercase() == lower)
            {
                continue;
            }
            terms.push(token.to_string());
            if terms.len() >= MAX_TERMS {
                return Some(terms);
            }
        }
    }

    if terms.is_empty() {
        None
    } else {
        Some(terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_requests_need_a_rewrite() {
        assert!(needs_rewrite("然后 后停 停来"));
        assert!(needs_rewrite("错误处理和限流回退的代码在哪里"));
    }

    #[test]
    fn requests_that_already_name_code_do_not_need_a_rewrite() {
        assert!(!needs_rewrite("where is retry handled"));
        assert!(!needs_rewrite("run_codebase_search 怎么改"));
        assert!(!needs_rewrite("看看 CodebaseSearch 的实现"));
    }

    #[test]
    fn parses_numbered_and_quoted_answers() {
        let terms =
            parse_rewritten_terms("1. retry_logic\n2) \"error_handler\"\n- backoff").unwrap();
        assert_eq!(terms, vec!["retry_logic", "error_handler", "backoff"]);
    }

    #[test]
    fn drops_prose_and_short_tokens() {
        let terms = parse_rewritten_terms("Sure! here you go: timeout, a, the, retry").unwrap();
        assert_eq!(terms, vec!["timeout", "retry"]);
    }

    #[test]
    fn empty_or_useless_answers_are_rejected() {
        assert!(parse_rewritten_terms("").is_none());
        assert!(parse_rewritten_terms("I cannot help with that").is_none());
    }

    #[test]
    fn deduplicates_case_insensitively() {
        let terms = parse_rewritten_terms("Retry retry RETRY").unwrap();
        assert_eq!(terms, vec!["Retry"]);
    }

    /// 现场事故：模型回了一句 `None`（意思是"没有可用的英文词"），解析器把它
    /// 当成合法检索词，于是拿 `None` 去搜英文代码库，捞回一片 `none`。
    #[test]
    fn meta_answers_are_not_search_terms() {
        assert!(parse_rewritten_terms("None").is_none());
        assert!(parse_rewritten_terms("null\nunknown\nnothing").is_none());
        assert!(parse_rewritten_terms("N/A").is_none());
    }

    /// 但"元回答"里混着真词时，真词要留下。
    #[test]
    fn real_terms_survive_next_to_a_meta_answer() {
        let terms = parse_rewritten_terms("None\nretry_backoff").unwrap();
        assert_eq!(terms, vec!["retry_backoff"]);
    }
}
