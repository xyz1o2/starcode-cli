//! StarLens 代码认知层：数据模型与紧凑文本格式。
//!
//! 本模块只定义"一条认知 entry 长什么样"以及它与磁盘上 `.star/lens/lens.txt`
//! 之间的序列化/反序列化，不涉及生成管线、注入或漂移检测（见 docs/starlens-design.md）。
//!
//! 设计要点：
//! - 符号标签（object/lang/role/sha）与 `symbols`/`imports` 来自解析器，永远正确；
//!   语义四元素 F/R/A/S 才来自 cheap model。
//! - 磁盘格式按字段分行、entry 间空行分隔，便于 LLM 单遍扫描，也便于 git diff。
//! - `token_cost` 是派生值（按 chars/4 估算），不持久化，加载时重算，避免陈旧。

use std::fmt::Write as _;

/// 受管对象在认知体系里的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeRole {
    /// 全认知：读内容、建 entry、进 Whole-Index。
    Index,
    /// 仅指纹：只跟踪 hash 测漂移，不建语义 entry。
    Observe,
    /// 跳过：认知引擎完全忽略。
    Exclude,
}

impl ScopeRole {
    fn as_str(self) -> &'static str {
        match self {
            ScopeRole::Index => "Index",
            ScopeRole::Observe => "Observe",
            ScopeRole::Exclude => "Exclude",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Index" => Some(ScopeRole::Index),
            "Observe" => Some(ScopeRole::Observe),
            "Exclude" => Some(ScopeRole::Exclude),
            _ => None,
        }
    }
}

/// 一条 StarLens 认知 entry。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensEntry {
    /// 受管对象路径（源文件相对路径；`db://schema.table` 预留给 P5）。
    pub object: String,
    /// 绑定的源内容指纹（完整 sha256 十六进制），漂移检测与 source-binding 用。
    pub source_sha256: String,
    /// 认知角色。
    pub role: ScopeRole,
    // ── 符号标签：解析器出，永不交给模型 ──
    pub lang: String,
    pub symbols: Vec<String>,
    pub imports: Vec<String>,
    // ── 语义四元素：仅此四项交给 cheap model ──
    pub f_role: String,
    pub r_related: Vec<String>,
    pub a_api: Vec<String>,
    pub s_signals: Vec<String>,
}

impl LensEntry {
    /// 按 chars/4 粗估 token 占用（对标 codemap/agentmap 的估法，±10%）。
    pub fn estimate_tokens(&self) -> u32 {
        let chars = self.to_block().chars().count();
        (chars / 4) as u32
    }

    /// 序列化成紧凑文本块（不含 entry 间的空行分隔）。
    pub fn to_block(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "@{} | {} | {} | #{}",
            self.object,
            self.lang,
            self.role.as_str(),
            self.source_sha256
        );
        // 列表字段：R/S/sym/imp 用逗号，A（API 签名内含逗号）用分号。
        let _ = writeln!(out, "F: {}", self.f_role);
        let _ = writeln!(out, "R: {}", self.r_related.join(", "));
        let _ = writeln!(out, "A: {}", self.a_api.join("; "));
        let _ = writeln!(out, "S: {}", self.s_signals.join(", "));
        let _ = writeln!(out, "sym: {}", self.symbols.join(", "));
        // 末行不带换行，交给调用方拼接。
        let _ = write!(out, "imp: {}", self.imports.join(", "));
        out
    }

    /// 从一个文本块解析。block 不含首尾空行。
    pub fn from_block(block: &str) -> Option<LensEntry> {
        let mut lines = block.lines();
        let header = lines.next()?.trim();
        let header = header.strip_prefix('@')?;
        let mut parts = header.split('|').map(|p| p.trim());
        let object = parts.next()?.to_string();
        let lang = parts.next()?.to_string();
        let role = ScopeRole::parse(parts.next()?)?;
        let sha = parts.next()?.trim_start_matches('#').to_string();

        let (mut f, mut r, mut a, mut s, mut sym, mut imp) =
            (String::new(), vec![], vec![], vec![], vec![], vec![]);
        for line in lines {
            if let Some(v) = line.strip_prefix("F:") {
                f = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("R:") {
                r = split_list(v, ',');
            } else if let Some(v) = line.strip_prefix("A:") {
                a = split_list(v, ';');
            } else if let Some(v) = line.strip_prefix("S:") {
                s = split_list(v, ',');
            } else if let Some(v) = line.strip_prefix("sym:") {
                sym = split_list(v, ',');
            } else if let Some(v) = line.strip_prefix("imp:") {
                imp = split_list(v, ',');
            }
        }
        Some(LensEntry {
            object,
            source_sha256: sha,
            role,
            lang,
            symbols: sym,
            imports: imp,
            f_role: f,
            r_related: r,
            a_api: a,
            s_signals: s,
        })
    }
}

/// 按分隔符切分并清洗：trim、去空项。
fn split_list(raw: &str, sep: char) -> Vec<String> {
    raw.split(sep)
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.to_string())
        .collect()
}

/// 磁盘 `lens.txt` 顶部的编码规则说明——让模型单遍即懂格式，无需外部文档。
pub const LENS_ENCODING_RULES: &str = "\
# StarLens Whole-Index. One entry per managed object; blank line between entries.
# Header: @<path> | <lang> | <role> | #<sha256>
# F=role/intent  R=related files  A=public API  S=high-signal keywords  sym=symbols  imp=imports
# Structural fields (header, sym, imp, A signatures) come from the parser; F/R/A/S semantics from a model.";

/// Whole-Index：编码规则 + 全部 Index 角色 entry，就是开工前注入模型的那份。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WholeIndex {
    pub entries: Vec<LensEntry>,
}

impl WholeIndex {
    /// 序列化成 `lens.txt` 的完整文本（规则头 + 空行分隔的 entry 块）。
    pub fn to_text(&self) -> String {
        let mut out = String::from(LENS_ENCODING_RULES);
        out.push_str("\n\n");
        let blocks: Vec<String> = self.entries.iter().map(|e| e.to_block()).collect();
        out.push_str(&blocks.join("\n\n"));
        out.push('\n');
        out
    }

    /// 从 `lens.txt` 文本解析。忽略以 `#` 开头的规则/注释行与空块。
    pub fn from_text(text: &str) -> WholeIndex {
        let mut entries = Vec::new();
        for raw_block in text.split("\n\n") {
            // 剥掉块内的注释行（规则头就是靠这一步被滤掉的）。
            let cleaned: String = raw_block
                .lines()
                .filter(|l| !l.trim_start().starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n");
            if cleaned.trim().is_empty() {
                continue;
            }
            if let Some(entry) = LensEntry::from_block(&cleaned) {
                entries.push(entry);
            }
        }
        WholeIndex { entries }
    }

    /// 本索引总 token 估算，用于注入预算把关。
    pub fn total_tokens(&self) -> u32 {
        self.entries.iter().map(|e| e.estimate_tokens()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LensEntry {
        LensEntry {
            object: "src/runtime/messages.rs".into(),
            source_sha256: "a1b2c3d4e5f6".into(),
            role: ScopeRole::Index,
            lang: "rs".into(),
            symbols: vec!["AgentRequest".into(), "StreamMessage".into()],
            imports: vec!["tokio::sync::mpsc".into()],
            f_role: "UI↔Agent 消息协议".into(),
            r_related: vec![
                "ui/services/worker.rs".into(),
                "ui/services/stream.rs".into(),
            ],
            a_api: vec!["enum AgentRequest{SendMessage,Abort}".into()],
            s_signals: vec!["mpsc".into(), "cap-100".into()],
        }
    }

    #[test]
    fn entry_block_round_trips() {
        let e = sample();
        let parsed = LensEntry::from_block(&e.to_block()).expect("parse");
        assert_eq!(e, parsed);
    }

    #[test]
    fn whole_index_round_trips_and_skips_rules_header() {
        let idx = WholeIndex {
            entries: vec![sample(), sample()],
        };
        let text = idx.to_text();
        assert!(text.contains("# StarLens Whole-Index"));
        let parsed = WholeIndex::from_text(&text);
        // 规则头被滤掉，只还原出 entry。
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed, idx);
    }

    #[test]
    fn empty_semantic_fields_survive_round_trip() {
        let mut e = sample();
        e.f_role.clear();
        e.r_related.clear();
        e.a_api.clear();
        e.s_signals.clear();
        let parsed = LensEntry::from_block(&e.to_block()).expect("parse");
        assert_eq!(e, parsed);
    }

    #[test]
    fn role_parsing_is_strict() {
        assert_eq!(ScopeRole::parse("Index"), Some(ScopeRole::Index));
        assert_eq!(ScopeRole::parse("Observe "), Some(ScopeRole::Observe));
        assert_eq!(ScopeRole::parse("nonsense"), None);
    }
}
