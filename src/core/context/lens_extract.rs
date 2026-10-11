//! StarLens 结构抽取：给定文件产出 `LensEntry` 的"结构半扇"（lang/symbols/imports/a_api）。
//!
//! 这半扇全部来自解析器（tree-sitter 符号 + 轻量 import 扫描），**永远正确**；
//! 语义半扇 F/R/S（以及对 a_api 的精选）由后续 cheap model 管线填充。

use super::lens::{LensEntry, ScopeRole};
use super::symbol::{self, SymbolKind};

const MAX_SYMBOLS: usize = 40;
const MAX_API: usize = 12;
const MAX_IMPORTS: usize = 24;

fn ext_of(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e).unwrap_or("")
}

/// ext → 语言桶（展示用标签）。未知扩展回显 ext 本身。
fn lang_bucket(ext: &str) -> String {
    let known = match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "react",
        "ts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "cxx" | "hxx" => "cpp",
        "toml" => "toml",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "sh" => "shell",
        "sql" => "sql",
        _ => "",
    };
    if known.is_empty() {
        ext.to_string()
    } else {
        known.to_string()
    }
}

/// ext → tree-sitter Language（镜像 call-graph 的解析器集合，保持本模块自洽）。
fn ts_language(ext: &str) -> Option<tree_sitter::Language> {
    match ext {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" | "pyi" => Some(tree_sitter_python::LANGUAGE.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" | "hpp" | "cc" | "cxx" | "hxx" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        _ => None,
    }
}

/// 产出结构骨架：object/sha/role/lang/symbols/imports/a_api 已填；
/// f_role/r_related/s_signals 留空，等 cheap model 管线填。
pub fn extract_structural(
    path: &str,
    content: &str,
    role: ScopeRole,
    source_sha256: String,
) -> LensEntry {
    let ext = ext_of(path);
    let (symbols, a_api) = extract_symbols_and_api(path, content, ext);
    let imports = extract_imports(content, ext);
    LensEntry {
        object: path.to_string(),
        source_sha256,
        role,
        lang: lang_bucket(ext),
        symbols,
        imports,
        a_api,
        f_role: String::new(),
        r_related: Vec::new(),
        s_signals: Vec::new(),
    }
}

/// 用 tree-sitter 抽符号名与 API 签名。无对应 grammar 时返回空。
fn extract_symbols_and_api(path: &str, content: &str, ext: &str) -> (Vec<String>, Vec<String>) {
    let Some(lang) = ts_language(ext) else {
        return (Vec::new(), Vec::new());
    };
    let mut base: symbol::SymbolId = 0;
    let Some(fs) = symbol::extract_symbols(content, path, ext, &lang, &mut base) else {
        return (Vec::new(), Vec::new());
    };

    let mut names: Vec<String> = Vec::new();
    // API 候选：公开面优先，私有兜底，各自保序去重。
    let mut public_api: Vec<String> = Vec::new();
    let mut other_api: Vec<String> = Vec::new();
    for s in &fs.symbols {
        if !s.name.is_empty() && !names.contains(&s.name) {
            names.push(s.name.clone());
        }
        let sig = trim_sig(&s.signature);
        if sig.is_empty() {
            continue;
        }
        if looks_public(&sig, s.kind) {
            if !public_api.contains(&sig) {
                public_api.push(sig);
            }
        } else if !other_api.contains(&sig) {
            other_api.push(sig);
        }
    }
    names.truncate(MAX_SYMBOLS);
    public_api.extend(other_api);
    public_api.truncate(MAX_API);
    (names, public_api)
}

fn trim_sig(sig: &str) -> String {
    let one_line = sig.lines().next().unwrap_or("").trim();
    if one_line.chars().count() > 100 {
        format!("{}…", one_line.chars().take(99).collect::<String>())
    } else {
        one_line.to_string()
    }
}

fn looks_public(sig: &str, kind: SymbolKind) -> bool {
    let s = sig.trim_start();
    s.starts_with("pub ")
        || s.starts_with("pub(")
        || s.starts_with("export ")
        || s.starts_with("public ")
        || matches!(
            kind,
            SymbolKind::Class | SymbolKind::Trait | SymbolKind::Interface | SymbolKind::Enum
        )
}

/// 轻量 import 扫描：按行识别各语言的导入语句，取模块路径。
/// 这是确定性结构事实；日后可升级为 tree-sitter 精确抽取。
fn extract_imports(content: &str, ext: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in content.lines() {
        if let Some(m) = import_module(raw.trim(), ext) {
            if !m.is_empty() && !out.contains(&m) {
                out.push(m);
            }
        }
        if out.len() >= MAX_IMPORTS {
            break;
        }
    }
    out
}

/// 单行 → 导入的模块路径（若该行是导入语句）。
fn import_module(line: &str, ext: &str) -> Option<String> {
    match ext {
        "rs" => line
            .strip_prefix("use ")
            .map(|r| r.trim_end_matches(';').trim().to_string()),
        "py" | "pyi" => {
            if let Some(rest) = line.strip_prefix("from ") {
                rest.split_once(" import ").map(|(m, _)| m.trim().to_string())
            } else {
                line.strip_prefix("import ")
                    .map(|r| r.split([' ', ',']).next().unwrap_or("").to_string())
            }
        }
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => {
            if line.starts_with("import ") || line.contains("require(") {
                quoted(line)
            } else {
                None
            }
        }
        "go" => {
            if line.starts_with("import ") || (line.starts_with('"') && line.ends_with('"')) {
                quoted(line)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 取一行里第一个单/双引号包起来的内容。
fn quoted(line: &str) -> Option<String> {
    for q in ['\'', '"'] {
        if let Some(start) = line.find(q) {
            if let Some(rel) = line[start + 1..].find(q) {
                return Some(line[start + 1..start + 1 + rel].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_structural_extraction() {
        let content = "use std::fmt;\npub struct Foo { x: i32 }\npub fn bar() {}\nfn helper() {}\n";
        let e = extract_structural("src/x.rs", content, ScopeRole::Index, "abc".into());
        assert_eq!(e.lang, "rust");
        assert!(e.symbols.contains(&"Foo".to_string()), "symbols={:?}", e.symbols);
        assert!(e.symbols.contains(&"bar".to_string()));
        assert!(e.imports.contains(&"std::fmt".to_string()), "imports={:?}", e.imports);
        assert!(!e.a_api.is_empty(), "a_api should capture public signatures");
        // 结构半扇填好，语义半扇留空待模型。
        assert!(e.f_role.is_empty() && e.r_related.is_empty() && e.s_signals.is_empty());
        assert_eq!(e.source_sha256, "abc");
        assert_eq!(e.role, ScopeRole::Index);
    }

    #[test]
    fn python_imports_and_symbols() {
        let content = "from os import path\nimport sys\n\ndef go():\n    return path\n";
        let e = extract_structural("m.py", content, ScopeRole::Index, "s".into());
        assert_eq!(e.lang, "python");
        assert!(e.imports.contains(&"os".to_string()), "imports={:?}", e.imports);
        assert!(e.imports.contains(&"sys".to_string()));
        assert!(e.symbols.contains(&"go".to_string()), "symbols={:?}", e.symbols);
    }

    #[test]
    fn unknown_ext_yields_lang_only() {
        let e = extract_structural("notes.txt", "hello world", ScopeRole::Observe, "s".into());
        assert_eq!(e.lang, "txt");
        assert!(e.symbols.is_empty());
        assert!(e.imports.is_empty());
        assert!(e.a_api.is_empty());
    }

    #[test]
    fn js_import_module_is_extracted() {
        let e = extract_structural(
            "a.ts",
            "import { foo } from './bar';\nexport function baz() {}\n",
            ScopeRole::Index,
            "s".into(),
        );
        assert_eq!(e.lang, "typescript");
        assert!(e.imports.contains(&"./bar".to_string()), "imports={:?}", e.imports);
    }
}

