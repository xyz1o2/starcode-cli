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

/// 提示词里最多带多少个**词表锚点**（仓库真实标识符）。
///
/// 锚点把"凭空猜"变成"挑现成的"：中文需求译出来的词必须**真的在索引里**
/// 才可能命中。把这个仓库自己的命名摆给模型，它就能选 `known_issues`，
/// 而不是猜一个这仓库根本没有的 `weakness`。
///
/// 对标 Windsurf FastContext 的 lexical anchoring（把本地词表注入检索提示）。
const MAX_ANCHORS: usize = 60;

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

/// 自动触发 CodebaseSearch 时的检索词构造。
///
/// 三种情况：
/// - **需求已经是代码侧的词**（英文 / 含标识符特征）→ 规则压缩，直接可用。
///   注意 `rewrite_search_query` 在这种情况下返回 `None`（"不需要改写"），
///   和"改写失败"的 `None` **不是一回事** —— 早期把两者混为一谈，
///   结果所有英文请求的自动触发都被跳过了。
/// - **需求是中文** → 必须改写成英文标识符；改写成功用改写结果。
/// - **中文且改写失败** → 返回 `None`（这一发不发）。
///   回退的 bigram 是中文词，索引里没有 → 必然零命中，还把模型带进
///   "再搜一遍同样的词"的死循环（现场：ARTEX，
///   `[ACE_REWRITE] query rewrite timed out after 8000ms` 之后发出
///   `CodebaseSearch(缺点)`，连着两次零命中）。
///
/// 少一次检索好过给模型一个必然为空的答案 —— 空结果会被它当成
/// 「这个功能不存在」。
pub(crate) async fn build_auto_search_query(
    client: &StarClient,
    user_input: &str,
) -> Option<String> {
    build_auto_search_query_in(client, user_input, None).await
}

/// 带项目根的版本（能带上词表锚点）。
pub(crate) async fn build_auto_search_query_in(
    client: &StarClient,
    user_input: &str,
    project_root: Option<&std::path::Path>,
) -> Option<String> {
    if !needs_rewrite(user_input) {
        // 已经是代码侧的词：规则压缩即可，不需要（也不该）花一次 LLM 调用。
        return Some(super::tool_call_builders::extract_search_keywords(
            user_input,
        ));
    }
    match rewrite_search_query_in(client, user_input, project_root).await {
        Some(terms) => Some(terms),
        None => {
            crate::utils::logging::append_debug_log_line(&format!(
                "[ACE_AUTO] query rewrite unavailable for a CJK request — skipping \
                 (fallback bigrams would be zero-hit): {user_input:?}"
            ));
            None
        }
    }
}

/// 取这个项目的**词表锚点**（真实标识符），给改写提示用。
///
/// 没有索引（冷启动）时返回空 vec —— 提示词照发，只是没有锚点。
fn anchors_for(project_root: Option<&std::path::Path>) -> Vec<String> {
    let Some(root) = project_root else {
        return Vec::new();
    };
    crate::core::context::search_cache::shared_root_identifier_vocabulary(root, MAX_ANCHORS)
        .unwrap_or_default()
}

/// 把需求改写成检索词；返回 `None` 表示"没改成"（调用方回退规则压缩）。
pub(crate) async fn rewrite_search_query(client: &StarClient, user_input: &str) -> Option<String> {
    rewrite_search_query_in(client, user_input, None).await
}

/// 带**词表锚点**的改写：把目标项目真实存在的标识符摆进提示词。
///
/// 没有锚点时，模型只能凭空译（"缺点" → `limitation`/`drawback`/`weakness`），
/// 而仓库实际用的可能��� `gap`/`known_issues`/`todo` —— 译出来的词索引里
/// 根本没有，命中率为零。有锚点时模型是在**挑**，不是在猜。
///
/// 对标 Windsurf FastContext 的 lexical anchoring。
pub(crate) async fn rewrite_search_query_in(
    client: &StarClient,
    user_input: &str,
    project_root: Option<&std::path::Path>,
) -> Option<String> {
    if !needs_rewrite(user_input) {
        return None;
    }

    let system = crate::types::StarMessage::system(
        "You translate a developer's request into code-search terms. Reply with ONLY a \
         space-separated list of at most 6 terms that would literally appear in source \
         code: identifiers (snake_case or CamelCase), type names, or technical nouns \
         in English. If a list of repository identifiers is given, prefer terms from it \
         — those are the words this codebase actually uses. If you cannot suggest any \
         plausible code term, reply with the single word NONE. No explanation, no \
         punctuation, no quotes.",
    );

    let anchors = anchors_for(project_root);
    let user = if anchors.is_empty() {
        crate::types::StarMessage::user(format!("Request: {user_input}\n\nTerms:"))
    } else {
        crate::types::StarMessage::user(format!(
            "Request: {user_input}\n\nRepository identifiers (prefer these when they fit):\n{}\n\nTerms:",
            anchors.join(", ")
        ))
    };

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

    /// 词表锚点从索引来 —— 冷启动时索引还没建好，锚点为空。
    #[test]
    fn the_anchor_list_is_empty_without_an_index() {
        let dir = tempfile::tempdir().expect("tmp");
        assert!(
            super::anchors_for(Some(dir.path())).is_empty(),
            "没有索引时不该编造词表"
        );
        assert!(
            super::anchors_for(None).is_empty(),
            "没有项目根时不该编造词表"
        );
    }

    /// 回归：**英文请求必须照常触发** CodebaseSearch。
    ///
    /// `rewrite_search_query` 对"不需要改写"的请求返回 `None`，而"改写失败"
    /// 也返回 `None` —— 两者不是一回事。早期把两者混为一谈，结果所有英文
    /// 请求的自动触发都被跳过了（英文是主要使用场景，等于把功能改没了）。
    #[tokio::test]
    async fn an_english_request_still_produces_a_query() {
        let client = crate::llm::client::StarClient::new("", None, None, None, None);
        let query = build_auto_search_query(&client, "where is retry handled")
            .await
            .expect("英文请求必须能构造出 query");
        assert!(
            query.contains("retry"),
            "英文请求的检索词应保留关键词，实际 {query:?}"
        );
    }

    /// 中文请求 + 改写不可用（没有可用的 provider）→ **不发**这一发。
    #[tokio::test]
    async fn a_cjk_request_without_a_rewrite_is_skipped() {
        let client = crate::llm::client::StarClient::new("", None, None, None, None);
        let query = build_auto_search_query(&client, "这个项目的缺点在哪").await;
        assert!(
            query.is_none(),
            "改写不可用时不该发这一发（回退 bigram 必然零命中），实际 {query:?}"
        );
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
