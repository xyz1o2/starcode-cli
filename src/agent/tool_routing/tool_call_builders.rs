use crate::types::StarToolCall;
use std::collections::HashSet;

pub(crate) fn build_analyzer_skill_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_skill_tool_call("analyzer", user_input, turn)
}

pub(crate) fn build_editor_skill_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_skill_tool_call("editor", user_input, turn)
}

pub(crate) fn build_navigator_skill_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_skill_tool_call("navigator", user_input, turn)
}

/// 构造 `CodebaseSearch` 调用，并把用户提问**压成关键词**。
///
/// # 为什么不能直接把整句提问塞进 `query`
///
/// 自动触发路径以前是 `{"query": user_input}`，也就是把"帮我看下错误处理在哪儿"
/// 这样的自然语言整句丢给一个 TF-IDF 词法引擎。而工具自己的 schema 明确要求
/// `"Keywords or a short phrase"`。后果是问句词（哪 / 帮我 / 一下 / how / does /
/// where）占掉大半信号权重，query expansion 还沿着这些噪声乱扩——**唯一的自动
/// 路径喂给了词法引擎最不适合的输入形态**。
///
/// 现在自动路径和模型手调路径的输入形态一致了：都是关键词。
pub(crate) fn build_codebase_search_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_tool_call(
        "CodebaseSearch",
        &serde_json::json!({"query": extract_search_keywords(user_input), "turn": turn}),
    )
}

/// 把自然语言提问压成给词法引擎用的关键词串。
///
/// 保留标识符（`snake_case`、`CamelCase`、`dotted.path`、`::`），丢掉问句词、
/// 助词、标点和过短的噪声词。压不出东西就原样返回——总比发一个空 query 好。
///
/// 中文没有空格，整句会被切成一个 token，虚词表就完全失效（`错误处理和限流回退的代码在哪里`
/// 是一个 token）。所以 CJK 连续段单独处理：逐字丢掉虚词，再把剩下的字拼成
/// 二元组（bigram）——中文关键词检索里 bigram 是无需分词器的稳妥近似，
/// 且比单字更抗噪（"处理"不会被"处"和"理"两个高频字拆散）。
pub(crate) fn extract_search_keywords(query: &str) -> String {
    /// 英文停用词 + 中文虚词。按切分后的 token 逐个比对。
    const STOPWORDS: &[&str] = &[
        // 疑问 / 指令
        "how",
        "what",
        "where",
        "which",
        "why",
        "who",
        "when",
        "does",
        "do",
        "did",
        "is",
        "are",
        "was",
        "were",
        "can",
        "could",
        "should",
        "would",
        "will",
        "shall",
        "may",
        "you",
        "your",
        "i",
        "me",
        "my",
        "we",
        "our",
        "please",
        "help",
        "want",
        "need",
        "let",
        "show",
        "tell",
        "find",
        "look",
        "search",
        "about",
        "some",
        "any",
        "all",
        "code",
        "codebase",
        "repo",
        "repository",
        "project",
        "file",
        "files",
        "thing",
        "stuff",
        "here",
        "there",
        "get",
        "got",
        "know",
        "like",
        // 连词 / 介词 / 指示词
        "and",
        "or",
        "not",
        "but",
        "the",
        "this",
        "that",
        "these",
        "those",
        "its",
        "it's",
        "with",
        "from",
        "into",
        "than",
        "then",
        "when",
        "while",
        "over",
        "under",
        "out",
        "off",
        "own",
        "same",
        "too",
        "very",
        "just",
        "also",
        "more",
        "most",
        "other",
        "such",
        // 中文虚词
        "的",
        "了",
        "吗",
        "呢",
        "吧",
        "啊",
        "我",
        "你",
        "他",
        "她",
        "它",
        "们",
        "这",
        "那",
        "哪",
        "个",
        "些",
        "有",
        "在",
        "是",
        "把",
        "被",
        "和",
        "与",
        "或",
        "就",
        "都",
        "也",
        "还",
        "很",
        "太",
        "请",
        "帮",
        "给",
        "下",
        "上",
        "中",
        "里",
        "外",
        "会",
        "要",
        "能",
        "用",
        "看",
        "查",
        "找",
        "做",
        "说",
        "怎",
        "么",
        "如",
        "何",
        "为",
        "什",
        "其",
    ];

    let mut kept: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |term: String, kept: &mut Vec<String>, seen: &mut HashSet<String>| {
        if term.is_empty() || kept.len() >= 8 {
            return;
        }
        if seen.insert(term.to_lowercase()) {
            kept.push(term);
        }
    };

    for raw in query.split_whitespace() {
        // 去掉包裹标点，但保留标识符内部的 `_ : .` 和大小写。
        let token = raw.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '_' && c != ':' && c != '.' && c != '#' && c != '@'
        });
        if token.is_empty() {
            continue;
        }

        // ── CJK 连续段：逐字滤虚词 + bigram ──
        let cjk_len = token.chars().filter(|c| is_cjk(*c)).count();
        if cjk_len * 2 >= token.chars().count() {
            let chars: Vec<char> = token
                .chars()
                .filter(|c| is_cjk(*c) && !STOPWORDS.contains(&c.to_string().as_str()))
                .collect();
            for pair in chars.windows(2) {
                push(pair.iter().collect(), &mut kept, &mut seen);
            }
            // 单字成词的场合（"锁"、"缓存"只有一个字命中时）保留末字兜底。
            if chars.len() == 1 {
                push(chars[0].to_string(), &mut kept, &mut seen);
            }
            continue;
        }

        // ── 拉丁 / 代码 ──
        let lower = token.to_lowercase();

        // 代码味道很重的 token（路径、`::`、`#tag`、`@path`）原样保留，它们本身就是关键词。
        let looks_like_code = token.contains("::")
            || token.contains('/')
            || token.contains('#')
            || token.starts_with('@')
            || token.contains('.')
            || token.contains('_')
            || token.chars().any(|c| c.is_uppercase());

        if !looks_like_code && (token.chars().count() < 3 || STOPWORDS.contains(&lower.as_str())) {
            continue;
        }
        // 纯停用词即使长得像代码（例如 `Where`）也不该进查询。
        if STOPWORDS.contains(&lower.as_str()) && token.chars().count() <= 5 {
            continue;
        }

        push(token.to_string(), &mut kept, &mut seen);
    }

    if kept.is_empty() {
        // 压不出关键词（纯虚词提问）——原样回传，让引擎自己去撞大炮。
        return query.trim().to_string();
    }
    kept.join(" ")
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

pub(crate) fn build_project_map_tool_call(user_input: &str, turn: i32) -> StarToolCall {
    build_tool_call(
        "ProjectMap",
        &serde_json::json!({"query": user_input, "turn": turn}),
    )
}

pub(crate) fn build_validation_tool_call(turn: i32) -> StarToolCall {
    build_tool_call("GetDiagnostics", &serde_json::json!({"turn": turn}))
}

pub(crate) fn build_json_fallback_prompt(
    content: &str,
    active_tools: &std::collections::HashSet<String>,
) -> String {
    let tools_str: Vec<&str> = active_tools.iter().map(|s| s.as_str()).collect();
    let tools_joined = tools_str.join(", ");
    format!(
        "Please respond with valid JSON. Active tools: {}\n\n{}",
        tools_joined, content
    )
}

pub(crate) fn json_fallback_extract_tool_call(response_text: &str) -> Option<StarToolCall> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(response_text) {
        if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
            let args = json
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            return Some(build_tool_call(name, &args));
        }
    }
    None
}

/// 构造 skill 工具调用。
///
/// 用户问题必须放进 `args.objective`——SkillTool 只读 `skill` 和 `args`，
/// 其它字段（包括旧的 `task` 和 `turn`）被 serde 静默丢弃。若放在 `task` 里，
/// 子代理的 objective 会回退成技能名本身（如把 "navigator" 当成语义查询）。
fn build_skill_tool_call(skill: &str, user_input: &str, _turn: i32) -> StarToolCall {
    build_tool_call(
        "Skill",
        &serde_json::json!({
            "skill": skill,
            "args": {"objective": user_input}
        }),
    )
}

fn build_tool_call(name: &str, args: &serde_json::Value) -> StarToolCall {
    StarToolCall {
        id: format!(
            "call_{}_{}",
            name,
            uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
        ),
        call_type: "function".to_string(),
        function: crate::types::StarToolCallFunction {
            name: name.to_string(),
            arguments: serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归 ACE-01：用户问题曾放在 `task` 字段里，而 SkillTool 只读
    /// `args.objective`，导致 navigator 子代理拿字符串 "navigator"
    /// 当成语义查询。真实问题必须出现在 objective 上。
    #[test]
    fn skill_call_carries_question_in_objective() {
        let call = build_navigator_skill_tool_call("where is auth handled", 3);
        assert_eq!(call.function.name, "Skill");

        let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
        assert_eq!(
            args.get("args")
                .and_then(|a| a.get("objective"))
                .and_then(|o| o.as_str()),
            Some("where is auth handled"),
            "user question must land in args.objective"
        );
        // 旧的泄漏字段不能还在
        assert!(
            args.get("task").is_none(),
            "stale `task` field must be gone"
        );
        assert!(
            args.get("turn").is_none(),
            "stale `turn` field must be gone"
        );
    }

    #[test]
    fn analyzer_and_editor_skill_calls_also_carry_objective() {
        for call in [
            build_analyzer_skill_tool_call("explain the loop", 1),
            build_editor_skill_tool_call("fix the bug", 1),
        ] {
            let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
            let objective = args
                .get("args")
                .and_then(|a| a.get("objective"))
                .and_then(|o| o.as_str());
            assert!(objective.is_some_and(|o| !o.is_empty() && o != "analyzer" && o != "editor"));
        }
    }

    // ── 自动触发的 query 形态 ──────────────────────────────────────────────
    //
    // 回归：自动触发以前把整句自然语言塞进 `query`，而 CodebaseSearch 是词法
    // TF-IDF 引擎、schema 明确要求 keywords。问句词会稀释排序信号。

    #[test]
    fn codebase_search_call_sends_keywords_not_the_raw_question() {
        let call = build_codebase_search_tool_call(
            "where is retry and backoff handled in this codebase?",
            2,
        );
        let args: serde_json::Value = serde_json::from_str(&call.function.arguments).unwrap();
        let q = args.get("query").and_then(|q| q.as_str()).unwrap();

        assert_ne!(q, "where is retry and backoff handled in this codebase?");
        assert!(q.contains("retry"), "关键词丢了实词：{q}");
        assert!(q.contains("backoff"), "关键词丢了实词：{q}");
        for noise in ["where", "codebase", "this", "in", "and"] {
            assert!(
                !q.to_lowercase().split_whitespace().any(|w| w == noise),
                "问句词 `{noise}` 没被剔除：{q}"
            );
        }
        // "handled" 是内容词而不是问句词——词法引擎里它是有用的检索词，保留它是对的。
        assert!(q.contains("handled"), "误删了内容词：{q}");
        assert!(q.chars().count() < 45, "query 仍然过长，等于没压缩：{q}");
    }

    #[test]
    fn codebase_search_keeps_identifiers_whole() {
        let q =
            extract_search_keywords("look at run_codebase_search_for_skill in tool_call_builders");
        assert!(q.contains("run_codebase_search_for_skill"), "{q}");
        assert!(q.contains("tool_call_builders"), "{q}");
    }

    #[test]
    fn codebase_search_keeps_chinese_content_words() {
        // 中文没有空格分词：整句是一个 token，必须走 CJK bigram 分支，
        // 否则虚词表完全失效、问句尾巴「在哪里」会被当成关键词发给引擎。
        let q = extract_search_keywords("错误处理和限流回退的代码在哪里");
        assert!(q.contains("错误"), "{q}");
        assert!(q.contains("处理"), "{q}");
        assert!(q.contains("限流"), "{q}");
        assert!(q.contains("回退"), "{q}");
        assert!(!q.contains("在哪"), "问句尾巴没被剔除：{q}");
        assert!(!q.contains("的"), "虚词没被剔除：{q}");
    }

    #[test]
    fn codebase_search_mixed_cjk_and_identifiers() {
        let q = extract_search_keywords("retry 机制在 run_codebase_search 里怎么实现的？");
        assert!(q.contains("retry"), "{q}");
        assert!(q.contains("run_codebase_search"), "{q}");
        assert!(q.contains("机制"), "{q}");
    }

    #[test]
    fn codebase_search_dedupes_and_caps_terms() {
        let q = extract_search_keywords(
            "cache cache cache retry retry budget budget limit limit token token session session trace trace span span",
        );
        let words: Vec<&str> = q.split_whitespace().collect();
        let mut unique = words.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), words.len(), "关键词未去重：{q}");
        assert!(words.len() <= 8, "关键词没截断：{q}");
    }

    /// 压不出关键词时原样回传——发空 query 比发一句噪声更糟。
    #[test]
    fn codebase_search_falls_back_to_raw_input_when_nothing_survives() {
        assert_eq!(extract_search_keywords("what is it"), "what is it");
        assert_eq!(extract_search_keywords("????"), "????");
    }
}
