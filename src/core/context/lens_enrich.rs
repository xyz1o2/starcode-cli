//! StarLens 语义增强：给结构 entry 填 F/R/S（职责/关系/信号）。
//!
//! 确定性部分（prompt 构造、响应解析、回填）在本模块，可用 mock 单测；
//! 实时模型调用藏在 `SemanticEnricher` trait 之后，由适配器接现有 LLM client。
//! 任一步失败一律**诚实降级**为结构-only，绝不编造语义。

use super::lens::LensEntry;
use crate::llm::client::StarClient;

/// cheap model 产出的语义三元素（a_api 的精选留给结构层，模型只补 F/R/S）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Semantics {
    pub f_role: String,
    pub r_related: Vec<String>,
    pub s_signals: Vec<String>,
}

/// 发给模型的内容截断上限：F/R/S 看个大概即可，不必灌全文。
const MAX_CONTENT_CHARS: usize = 6000;

/// 构造增强 prompt：结构骨架 + 截断内容，要求严格三行输出。
pub fn build_prompt(entry: &LensEntry, content: &str) -> String {
    let body = content.chars().take(MAX_CONTENT_CHARS).collect::<String>();
    format!(
        "You are indexing one source file for a code-cognition map. Be terse and factual.\n\
         File: {object}\nLang: {lang}\nSymbols: {syms}\n\n\
         Return EXACTLY three lines, nothing else:\n\
         F: <one sentence: this file's responsibility/intent>\n\
         R: <comma-separated repo-relative paths of the few most related files; [] if none>\n\
         S: <comma-separated dense keywords of high-signal implementation details; [] if none>\n\n\
         ----- FILE CONTENT (may be truncated) -----\n{body}",
        object = entry.object,
        lang = entry.lang,
        syms = entry.symbols.join(", "),
    )
}

/// 解析模型响应里的 F/R/S 三行。宽松：忽略多余行、允许缺失。
pub fn parse_semantics(resp: &str) -> Semantics {
    let mut s = Semantics::default();
    for line in resp.lines() {
        let t = line.trim();
        if let Some(v) = t.strip_prefix("F:") {
            s.f_role = v.trim().to_string();
        } else if let Some(v) = t.strip_prefix("R:") {
            s.r_related = parse_list(v);
        } else if let Some(v) = t.strip_prefix("S:") {
            s.s_signals = parse_list(v);
        }
    }
    s
}

fn parse_list(v: &str) -> Vec<String> {
    let v = v.trim();
    if v.is_empty() || v == "[]" {
        return Vec::new();
    }
    v.trim_matches(['[', ']'].as_ref())
        .split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect()
}

/// 把语义回填到结构 entry（结构字段不动）。
pub fn apply(entry: &mut LensEntry, sem: Semantics) {
    entry.f_role = sem.f_role;
    entry.r_related = sem.r_related;
    entry.s_signals = sem.s_signals;
}

/// 语义增强器：实现方负责真正调模型。失败/不可用返回 `None`。
pub trait SemanticEnricher {
    fn enrich(&self, prompt: &str) -> Option<String>;
}

/// 对一个 entry 执行完整增强：构造 prompt → 调模型 → 解析 → 回填。
/// 任一步失败则 entry 保持结构-only，返回 `false`（诚实降级）。
pub fn enrich_entry(entry: &mut LensEntry, content: &str, enricher: &dyn SemanticEnricher) -> bool {
    let prompt = build_prompt(entry, content);
    let Some(resp) = enricher.enrich(&prompt) else {
        return false;
    };
    let sem = parse_semantics(&resp);
    // 没拿到最关键的职责 F，视为失败，不回填。
    if sem.f_role.is_empty() {
        return false;
    }
    apply(entry, sem);
    true
}

/// 实时增强路径：用现有 `StarClient` 对一个 entry 调一次 cheap 补全。
/// 复用已测的 `build_prompt`/`parse_semantics`/`apply`；失败一律诚实降级返回 false。
pub async fn enrich_entry_with_client(
    entry: &mut LensEntry,
    content: &str,
    client: &StarClient,
) -> bool {
    let prompt = build_prompt(entry, content);
    match client.chat_completion_simple(&prompt).await {
        Ok(resp) => {
            let sem = parse_semantics(&resp);
            if sem.f_role.is_empty() {
                return false;
            }
            apply(entry, sem);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::lens::ScopeRole;

    struct Mock(Option<String>);
    impl SemanticEnricher for Mock {
        fn enrich(&self, _prompt: &str) -> Option<String> {
            self.0.clone()
        }
    }

    fn structural_entry() -> LensEntry {
        LensEntry {
            object: "src/runtime/messages.rs".into(),
            source_sha256: "abc".into(),
            role: ScopeRole::Index,
            lang: "rust".into(),
            symbols: vec!["AgentRequest".into()],
            imports: vec![],
            f_role: String::new(),
            r_related: vec![],
            a_api: vec![],
            s_signals: vec![],
        }
    }

    #[test]
    fn prompt_carries_structure_and_truncates() {
        let e = structural_entry();
        // 用 '9' 做标记字符：固定模板里没有数字，便于精确计数被截断后的内容量。
        let big = "9".repeat(MAX_CONTENT_CHARS + 500);
        let p = build_prompt(&e, &big);
        assert!(p.contains("File: src/runtime/messages.rs"));
        assert!(p.contains("AgentRequest"));
        assert_eq!(p.matches('9').count(), MAX_CONTENT_CHARS);
    }

    #[test]
    fn parse_handles_lists_and_empties() {
        let s = parse_semantics("F: protocol hub\nR: a.rs, b.rs\nS: []\n");
        assert_eq!(s.f_role, "protocol hub");
        assert_eq!(s.r_related, vec!["a.rs".to_string(), "b.rs".to_string()]);
        assert!(s.s_signals.is_empty());
    }

    #[test]
    fn enrich_fills_entry_on_good_response() {
        let mut e = structural_entry();
        let m = Mock(Some(
            "F: UI↔Agent 协议\nR: worker.rs\nS: mpsc, cap-100\n".into(),
        ));
        assert!(enrich_entry(&mut e, "code", &m));
        assert_eq!(e.f_role, "UI↔Agent 协议");
        assert_eq!(e.r_related, vec!["worker.rs".to_string()]);
        assert_eq!(e.s_signals, vec!["mpsc".to_string(), "cap-100".to_string()]);
    }

    #[test]
    fn enrich_degrades_honestly_on_failure() {
        // 模型不可用 → false，entry 语义半扇保持空。
        let mut e = structural_entry();
        assert!(!enrich_entry(&mut e, "code", &Mock(None)));
        assert!(e.f_role.is_empty());
        // 响应缺 F → 也视为失败，不回填。
        let mut e2 = structural_entry();
        assert!(!enrich_entry(
            &mut e2,
            "code",
            &Mock(Some("S: foo\n".into()))
        ));
        assert!(e2.f_role.is_empty() && e2.s_signals.is_empty());
    }
}
