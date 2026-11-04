//! 工具注册名与实现的一致性守卫。
//!
//! 工具名散落在两处：枚举 [`crate::core::tools::constants::ToolName`] 是**声明**，
//! 各工具的 `fn name()` 是**实现**。两边漂移了不会编译失败，只会在运行时
//! 让模型调到一个不存在的工具 —— 表现为「工具明明在 schema 里，却报
//! tool not registered」。
//!
//! 这个测试扫源码里 `fn name()` 后紧跟的字符串字面量，断言每个都是合法注册名。
//! 静态扫描而非运行时枚举，是因为构造完整的 `ToolRegistry` 需要 LLM client
//! 与配置，测试环境里代价太高；而漂移恰恰是**静态**就能发现的问题。

use crate::core::tools::constants::ToolName;

/// 已知合法、但不由 `fn name()` 字面量提供的注册名。
///
/// 这些工具的名字来自别处（构造参数、枚举的 `as_str()`），
/// 静态扫不到它们 —— 在这里列出，是为了让"扫到的都合法"这条断言
/// 不至于因为漏扫而给出虚假的安全感。
const NAMES_ELSEWHERE: &[&str] = &[
    "TodoWrite",      // tasks.rs：构造时传入
    "LSP",            // tools/lsp/mod.rs：ToolName::LSP.as_str()
    "GetDiagnostics", // tools/diagnostics.rs：ToolName::GetDiagnostics.as_str()
];

/// UI 里出现、但**语义不是工具名**的字面量。
///
/// 扫 match arm 时会撞上这些：它们的 arm 标签长得像工具名别名（带下划线），
/// 但实际是任务状态之类的枚举值。不归"工具名一致"这条守管。
const NON_TOOL_NAME_LITERALS: &[&str] = &[
    "in_progress", // TodoWrite 的 status 值
    "completed",   // TodoWrite 的 status 值
];

/// 提取源码里 `fn name(&self) -> &str` 后面紧跟的字符串字面量。
///
/// 只认「函数签名 + 空行 + 立即返回字面量」这一种形态 —— 这是本仓库里
/// 工具实现 `name()` 的统一写法。其它形态（返回 `&self.name`、返回
/// `ToolName::X.as_str()`）由 `NAMES_ELSEWHERE` 与枚举本身覆盖。
fn extract_declared_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if !trimmed.starts_with("fn name(&self) -> &str") {
            continue;
        }
        // 函数体可能跨行；往下看几行，找到第一个字符串字面量就停
        for body_line in lines.clone().take(3) {
            let body = body_line.trim();
            if body.starts_with('"') && body.ends_with('"') && body.len() >= 2 {
                names.push(body.trim_matches('"').to_string());
                break;
            }
            // 命中了别的语句形态就放弃这一处，不猜
            if body.starts_with('&') || body.starts_with("ToolName::") {
                break;
            }
        }
    }
    names
}

/// 收集本仓库里所有工具实现声明的名字。
fn collect_all_declared_names() -> Vec<(String, String)> {
    let mut out = Vec::new();
    // 工具实现分布在两个目录：`core/tools`（大部分）与 `tools`（薄封装）
    for dir in ["src/core/tools", "src/tools"] {
        collect_from_dir(std::path::Path::new(dir), &mut out);
    }
    out
}

/// 扫描 UI 层里"按工具名匹配"的字符串字面量。
///
/// 返回的是**全部** arm 字面量（合法的也返回），规则由调用方判定 ——
/// 这样"扫到了东西"这件事本身才能被验证（扫不到 = 扫描坏了 = 空转）。
///
/// 为什么单独扫：`fn name()` 扫描只看工具实现，**看不到 UI 的 match arm** ——
/// 而 UI 里残留的旧别名（`"WebFetch" | "web_fetch"`）就是这么活下来的：
/// 改名时改了实现、忘了改渲染分支，那个分支从此命中不了，死得无声无息。
///
/// 扫的文件：`src/ui/components`、`src/ui/utils` 下文件名含 tool 或等于 format 的。
/// 工具名的 match arm 就住在这几个文件里；扫全 UI 会撞上 badge 之类的无关 arm。
fn collect_ui_tool_name_literals() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in ["src/ui/components", "src/ui/utils"] {
        collect_ui_names_from_dir(std::path::Path::new(dir), &mut out);
    }
    out
}

/// 判定一个字面量是不是合法的"工具名写法"。
///
/// 合法当且仅当它是
/// - 注册名本身（`WebFetch` / `TodoWrite`），或
/// - 非工具名的同形字面量（TodoWrite 的 status 值 `in_progress`）。
///
/// **注册名一律 PascalCase**（项目约定，见 `tool_name_consistency` 的既有断言
/// "工具名不该带下划线"）。大小写漂移由入口的
/// [`crate::core::tools::constants::resolve_registered_tool_name`] 一次性纠正，
/// arm 里不需要（也不应该）写小写变体。
///
/// 插件工具名与 MCP 工具名（`complete_task`、`mcp__server__tool`）不在注册名
/// 枚举里，但**也不该**出现在按工具名匹配的 arm 上 —— 它们没有固定形态，
/// 按名字猜参数只会误判。要展示它们就原样显示（`_ => name`）。
fn is_legal_tool_name_literal(literal: &str) -> bool {
    NON_TOOL_NAME_LITERALS.contains(&literal) || ToolName::from_str(literal).is_some()
}

fn collect_ui_names_from_dir(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ui_names_from_dir(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        // 只在"工具渲染"文件里扫：文件名含 tool 或等于 format。
        // 工具名的 match arm 就住在这几个文件里；扫全 UI 会撞上 badge 之类的无关 arm。
        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !(file_stem.contains("tool") || file_stem == "format") {
            continue;
        }
        for (lineno, line) in source.lines().enumerate() {
            // 只看 match arm 的标签位置。一行里可能有多个字面量
            // （`"WebFetch" | "web_search" => {`），**每一个**都要看 ——
            // 只看第一个会漏掉 arm 里的后续别名。
            let trimmed = line.trim_start();
            if !trimmed.starts_with('"') {
                continue;
            }
            let mut rest = trimmed;
            let mut literals: Vec<String> = Vec::new();
            loop {
                let Some(after_quote) = rest.strip_prefix('"') else {
                    break;
                };
                let Some(end) = after_quote.find('"') else {
                    break;
                };
                literals.push(after_quote[..end].to_string());
                rest = after_quote[end + 1..].trim_start();
                if let Some(tail) = rest.strip_prefix('|') {
                    rest = tail.trim_start();
                    continue;
                }
                break;
            }
            // 必须是 arm 标签：所有字面量之后跟 `=>`。
            if literals.is_empty() || !rest.starts_with("=>") {
                continue;
            }
            for literal in literals {
                out.push((literal, format!("{}:{}", path.display(), lineno + 1)));
            }
        }
    }
}

fn collect_from_dir(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_from_dir(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for name in extract_declared_names(&source) {
            out.push((name, path.display().to_string()));
        }
    }
}

/// 注册名 → 它改名之前的 snake_case 写法。
///
/// `WebFetch` → `web_fetch`，`TodoWrite` → `todo_write`，`ReadManyFiles` →
/// `read_many_files`。这些是**历史拼法**，不是别名 —— `canonical_tool_name`
/// 对它们原样返回，执行器也就 `get_tool()` 不到。
fn legacy_snake_case_spelling(registered: &str) -> String {
    let mut out = String::new();
    for (i, ch) in registered.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// 扫描整个 `src/`，找"旧 snake_case 拼法被当成工具名用"的地方。
///
/// 匹配范围：字符串字面量出现在行首（match arm 标签 / 常量定义）。
///
/// 跳过测试区域：`mod tests` 里的断言**故意**提到旧拼法（证明它不是别名），
/// 那不是"当成工具名用"。
fn collect_legacy_alias_usages() -> Vec<(String, String)> {
    let legacy: Vec<(String, String)> = ToolName::all_builtin()
        .iter()
        .map(|t| t.as_str())
        .map(|name| (legacy_snake_case_spelling(name), name.to_string()))
        .filter(|(snake, _)| snake.contains('_'))
        .collect();

    let mut out = Vec::new();
    for dir in ["src"] {
        collect_legacy_from_dir(std::path::Path::new(dir), &legacy, &mut out);
    }
    out
}

fn collect_legacy_from_dir(
    dir: &std::path::Path,
    legacy: &[(String, String)],
    out: &mut Vec<(String, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_legacy_from_dir(&path, legacy, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (lineno, line) in non_test_lines(&source) {
            let trimmed = line.trim_start();
            // 只看行首的字面量（match arm 标签 / 常量定义）。
            let Some(rest) = trimmed.strip_prefix('"') else {
                continue;
            };
            let Some(end) = rest.find('"') else {
                continue;
            };
            let literal = &rest[..end];
            if let Some((_, registered)) = legacy.iter().find(|(snake, _)| snake == literal) {
                out.push((
                    literal.to_string(),
                    format!("{}:{} (注册名 {registered})", path.display(), lineno + 1),
                ));
            }
        }
    }
}

/// 去掉测试专用区域里的行，返回 `(行号, 行内容)`。
///
/// 判据：`mod <名字>` 后面带 `test`（`mod tests`、`mod alias_tests`）的模块
/// 整块跳过 —— 那里的字符串字面量是**断言内容**，不是"当成工具名用"。
/// 跟踪花括号深度找模块的结束位置。
fn non_test_lines(source: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut test_module_end_depth: Option<i32> = None;
    for (idx, line) in source.lines().enumerate() {
        if let Some(end_depth) = test_module_end_depth {
            if depth < end_depth {
                test_module_end_depth = None;
            } else {
                depth += line.matches('{').count() as i32;
                depth -= line.matches('}').count() as i32;
                continue;
            }
        }
        let trimmed = line.trim_start();
        // `mod tests {` / `mod alias_tests {`
        let is_test_module =
            trimmed.starts_with("mod ") && trimmed.contains("test") && trimmed.ends_with('{');
        if is_test_module {
            test_module_end_depth = Some(depth);
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            continue;
        }
        out.push((idx + 1, line));
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
    }
    out
}

#[test]
fn every_declared_tool_name_is_a_registered_name() {
    let declared = collect_all_declared_names();
    assert!(
        !declared.is_empty(),
        "没扫到任何工具名 —— 扫描逻辑本身坏了，测试会变成空转"
    );

    let mut unknown = Vec::new();
    for (name, path) in &declared {
        if ToolName::from_str(name).is_none() {
            unknown.push(format!("{name} ({path})"));
        }
    }
    assert!(
        unknown.is_empty(),
        "这些工具的 name() 不在 ToolName 枚举里 —— 声明与实现漂移了：\n  {}",
        unknown.join("\n  ")
    );
}

#[test]
fn no_declared_name_uses_the_legacy_snake_case_spelling() {
    let declared = collect_all_declared_names();
    let legacy: Vec<String> = declared
        .iter()
        .filter(|(name, _)| name.contains('_'))
        .map(|(name, path)| format!("{name} ({path})"))
        .collect();
    assert!(
        legacy.is_empty(),
        "工具名不该带下划线（对标 Claude Code 的 PascalCase）：\n  {}",
        legacy.join("\n  ")
    );
}

#[test]
fn the_special_names_elsewhere_are_still_registered() {
    // 这几个的名字不来自 `fn name()` 字面量，容易在改名时漏掉
    for name in NAMES_ELSEWHERE {
        assert!(ToolName::from_str(name).is_some(), "{name} 应当仍是注册名");
    }
}

/// 全仓不该再把旧 snake_case 拼法当成工具名用。
///
/// 注册名是 PascalCase（`WebFetch`），旧拼法 `web_fetch` **不是别名**：
/// `canonical_tool_name` 对它原样返回，执行器也就 `get_tool()` 不到它。
/// 留着只会让人以为"这些写法都支持"，而实际是命中不了的死分支。
///
/// 这条守的是"改名时漏改匹配点"：实现改了、匹配点没改 → 分支从此命中不了，
/// 界面表现是折叠摘要悄悄退化成通用预览，不报任何错。
#[test]
fn no_legacy_snake_case_tool_name_is_used_anywhere() {
    let usages = collect_legacy_alias_usages();
    assert!(
        usages.is_empty(),
        "这些地方还在用旧 snake_case 工具名写法（命中不了的死分支）：\n  {}",
        usages
            .iter()
            .map(|(name, path)| format!("{name} ({path})"))
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// 工具名只有一个真相源：`ToolName` 枚举。
///
/// 以前还有一个 `src/constants/tools.rs`，里面是一份手抄的工具名清单 ——
/// 零用户、而且抄错了（`TOOL_LS = "ListDir"`，而注册名早就改成 `LS`）。
/// 两份清单并存时，改名只改一份，另一份就静默过期。
#[test]
fn there_is_no_second_handwritten_tool_name_list() {
    let stale = std::path::Path::new("src/constants/tools.rs");
    assert!(
        !stale.exists(),
        "src/constants/tools.rs 是手抄的工具名清单（零用户、值还是错的）—— \
         工具名只有一个真相源：core::tools::constants::ToolName"
    );
}
///
/// 这条守的是"改名时漏改渲染分支"：工具实现从 `web_fetch` 改成 `WebFetch` 之后，
/// UI 里 `"WebFetch" | "web_fetch"` 的后半个就成了死分支 —— 命中不了，也不报错，
/// 折叠摘要悄悄退化成通用预览。`fn name()` 扫描看不到 match arm，
/// 所以这条单独扫 UI 目录。
#[test]
fn ui_tool_name_match_arms_use_registered_names() {
    let literals = collect_ui_tool_name_literals();
    assert!(
        !literals.is_empty(),
        "没扫到任何 UI 工具名字面量 —— 扫描逻辑本身坏了，测试会变成空转"
    );

    let unknown: Vec<String> = literals
        .iter()
        .filter(|(name, _)| !is_legal_tool_name_literal(name))
        .map(|(name, path)| format!("{name} ({path})"))
        .collect();
    assert!(
        unknown.is_empty(),
        "UI 里这些工具名不是注册名 —— 多半是改名时漏掉的旧别名：\n  {}",
        unknown.join("\n  ")
    );
}

/// UI 层不该再出现 snake_case 的内置工具别名。
///
/// 注册名是 PascalCase（`WebFetch`），snake_case 写法（`web_fetch`）不是别名，
/// 而是拼错的注册名 —— `canonical_tool_name` 对它原样返回，执行器也就找不到
/// 工具。这类分支只能是死代码。
#[test]
fn ui_tool_name_match_arms_have_no_snake_case_builtin_aliases() {
    let literals = collect_ui_tool_name_literals();
    assert!(
        !literals.is_empty(),
        "没扫到任何 UI 工具名字面量 —— 扫描逻辑本身坏了，测试会变成空转"
    );
    // 只查**内置工具**的旧别名：任务状态值（`in_progress`）带下划线，
    // 但它不是工具名。
    let legacy: Vec<String> = literals
        .iter()
        .filter(|(name, _)| name.contains('_'))
        .filter(|(name, _)| !NON_TOOL_NAME_LITERALS.contains(&name.as_str()))
        .map(|(name, path)| format!("{name} ({path})"))
        .collect();
    assert!(
        legacy.is_empty(),
        "UI 里不该有带下划线的内置工具别名：\n  {}",
        legacy.join("\n  ")
    );
}

#[cfg(test)]
mod runtime_probe {
    use crate::core::config::runtime_bootstrap::build_core_tool_registry;
    use crate::core::config::{Config, ConfigParameters};
    use crate::core::confirmation_bus::MessageBus;
    use crate::core::policy::policy_engine::PolicyEngine;
    use crate::core::policy::types::PolicyEngineConfig;
    use crate::core::state::GlobalState;
    use std::sync::Arc;

    /// 运行时注册表里的工具名必须都是 PascalCase 的注册名。
    ///
    /// 这是比源码扫描更强的一层：它验的是**真正注册进 registry 的名字**，
    /// 而不是源码里写的字面量 —— 少一层中间环节，就少一种漂移。
    #[tokio::test]
    async fn registered_tools_use_the_canonical_pascal_case_names() {
        let config = Arc::new(Config::new(ConfigParameters::default()));
        let bus = Arc::new(MessageBus::new(
            PolicyEngine::new(PolicyEngineConfig {
                rules: None,
                checkers: None,
                hook_checkers: None,
                default_decision: None,
                non_interactive: None,
                allow_hooks: None,
                approval_mode: None,
            }),
            false,
        ));
        let gs = Arc::new(GlobalState::new());
        let reg = build_core_tool_registry(&config, config.clone(), bus, gs)
            .await
            .unwrap();
        let mut names: Vec<String> = reg
            .get_all_tool_entries()
            .into_iter()
            .map(|(n, _, _)| n)
            .collect();
        names.sort();
        assert!(!names.is_empty(), "注册表是空的 —— 测试会变成空转");

        for name in &names {
            assert!(
                crate::core::tools::constants::ToolName::from_str(name).is_some(),
                "注册表里的 {name} 不是合法注册名"
            );
            assert!(
                !name.contains('_'),
                "注册表里的 {name} 带下划线，应写 PascalCase"
            );
        }
    }
}
