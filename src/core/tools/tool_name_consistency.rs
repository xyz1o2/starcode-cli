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
