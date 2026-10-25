//! 从 settings.json 的 `hooks` 段读取 Claude Code 风格的 hook 定义。
//!
//! StarCode 自己的 hook 存在 `.star/hooks.json`（扁平数组），而 Claude Code 把
//! hooks 写在 settings.json 里，三层嵌套：
//!
//! ```json
//! {
//!   "hooks": {
//!     "PreToolUse": [
//!       { "matcher": "Bash",
//!         "hooks": [ { "type": "command", "command": "./guard.sh", "timeout": 30 } ] }
//!     ]
//!   }
//! }
//! ```
//!
//! 两者都读。settings.json 里的 hook **只读**（由用户自己维护，和 CC 一样），
//! `/hooks add|remove` 仍然只动 `.star/hooks.json` —— 所以导入项不���被
//! 误删，而本地新增的也不会污染用户提交进仓库的配置。
//!
//! 合并规则照抄 CC：**各层 hooks 累加而非互相替换**；同一个 handler 写进多个
//! settings 文件只跑一次。

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::store::{matcher_matches, ManagedHook, ManagedHookEvent};

/// CC 对 `command` 类 hook 的默认超时（秒）。`runner.rs` 另有 1000ms 下限。
const DEFAULT_COMMAND_HOOK_TIMEOUT_SECS: u64 = 600;

/// 参与读取的 settings 文件，从低优先级到高优先级。
///
/// 与 `PermissionRuleSet::candidate_files_with_trust` 的列表保持一致 —— 全局
/// `~/.star/settings.json` 管权限，hooks 不该走另一套，所以这里刻意复制而不是
/// 复用：两边的取舍不同（那边要 trust 分级，这边只要顺序）。
fn settings_files(cwd: &Path) -> Vec<PathBuf> {
    use crate::core::config::storage::Storage;
    let mut files = Vec::new();
    // 全局层：与 PermissionRuleSet 读同一个路径常量，两边不会各写一遍字符串。
    files.push(Storage::global_settings_path());
    let star_dir = Storage::new(cwd.to_path_buf()).star_dir();
    files.push(star_dir.join("settings.json"));
    files.push(star_dir.join("settings.local.json"));
    files
}

/// 解析单个 settings 文件里的 `hooks` 段。
///
/// 单条坏 entry 只跳过那一条并留下日志，不让整份文件失效 —— 与 CC 的
/// "Settings Warning: only individual entries fail" 行为一致。
fn parse_settings_file(path: &Path) -> Vec<ManagedHook> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    // settings 允许注释（StarCode 的 json_with_comments），复用同一个解析器。
    let Ok(root) =
        crate::core::config::json_with_comments::parse_json_with_comments::<Value>(&text)
    else {
        return Vec::new();
    };
    let Some(events) = root.get("hooks") else {
        return Vec::new();
    };
    let Some(events) = events.as_object() else {
        crate::utils::logging::append_debug_log_line(&format!(
            "[Hooks] {} 的 hooks 段不是对象，已忽略",
            path.display()
        ));
        return Vec::new();
    };

    let mut out = Vec::new();
    for (event_name, groups) in events {
        let Some(groups) = groups.as_array() else {
            continue;
        };

        // StarCode 没有的 CC 事件（PermissionRequest、PostCompact、WorktreeCreate
        // 等 20 多个）在这里被 parse 拒掉并跳过 —— 记一笔，避免用户以为配错了。
        let Some(event) = ManagedHookEvent::parse(event_name) else {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Hooks] {}: StarCode 暂无事件 `{}`，已跳过",
                path.display(),
                event_name
            ));
            continue;
        };

        for group in groups {
            let matcher = group
                .get("matcher")
                .and_then(|m| m.as_str())
                .map(|s| s.to_string());
            let Some(handlers) = group.get("hooks").and_then(|h| h.as_array()) else {
                continue;
            };

            for handler in handlers {
                match convert_handler(handler, event.clone(), &matcher, path) {
                    Ok(Some(hook)) => out.push(hook),
                    Ok(None) => {}
                    Err(reason) => {
                        crate::utils::logging::append_debug_log_line(&format!(
                            "[Hooks] {} 的 {}/{} 条目已跳过：{}",
                            path.display(),
                            event_name,
                            matcher.as_deref().unwrap_or("*"),
                            reason
                        ));
                    }
                }
            }
        }
    }
    out
}

/// 把一个 CC hook handler 转成 `ManagedHook`。
///
/// `Ok(None)` = 这条 handler 被静默跳过（非 command 类型，无可执行语义）。
/// `Err` = 配置有问题，值得让用户看到。
fn convert_handler(
    handler: &Value,
    event: ManagedHookEvent,
    matcher: &Option<String>,
    path: &Path,
) -> Result<Option<ManagedHook>, String> {
    let hook_type = handler
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("command")
        .to_string();

    // 只有 command 类能落到 ManagedHook：StarCode 的 runner 是 `sh -c <command>`。
    // http / prompt / agent / mcp_tool 需要各自的 executor，跳过并留痕。
    if hook_type != "command" {
        return Ok(None);
    }

    let Some(command) = handler.get("command").and_then(|c| c.as_str()) else {
        return Err("command 类 handler 缺少 command 字段".to_string());
    };
    if command.trim().is_empty() {
        return Err("command 为空".to_string());
    }

    // CC 的 exec form：有 args 时 command 是可执行文件、args 是参数数组，且不过
    // shell。StarCode 只有 `sh -c` 一条路，所以这里拼成一条 shell 命令串 ——
    // 对绝大多数脚本等价，但含空格/引号的路径需要用户自己加引号。
    let mut full = command.to_string();
    if let Some(args) = handler.get("args").and_then(|a| a.as_array()) {
        for arg in args.iter().filter_map(|a| a.as_str()) {
            full.push(' ');
            full.push_str(arg);
        }
    }

    let timeout_secs = handler
        .get("timeout")
        .and_then(|t| t.as_f64())
        .filter(|t| *t > 0.0)
        .map(|t| t as u64)
        .unwrap_or(DEFAULT_COMMAND_HOOK_TIMEOUT_SECS);

    let matcher_label = matcher.as_deref().unwrap_or("*");
    Ok(Some(ManagedHook {
        name: format!("settings:{}:{}", event.as_str(), matcher_label),
        event,
        command: full,
        timeout_secs,
        // CC 里 command hook 默认不阻塞：exit 0 无输出即"无意见"，放行到正常
        // 权限流程。blocking 只在 hook 自己 exit 2 时由 runner 处理。
        blocking: false,
        enabled: true,
        created_at: 0,
        updated_at: 0,
        source: Some(path.display().to_string()),
        working_dir: None,
        matcher: matcher.clone(),
    }))
}

/// 读取并合并全部 settings 文件的 hooks。
///
/// 按 (event, command) 去重：CC 明确写了"同一个 handler 在多个 settings 文件里
/// 定义只跑一次"。
pub fn load_settings_hooks(cwd: &Path) -> Vec<ManagedHook> {
    let mut out: Vec<ManagedHook> = Vec::new();
    for path in settings_files(cwd) {
        for hook in parse_settings_file(&path) {
            let dup = out
                .iter()
                .any(|existing| existing.event == hook.event && existing.command == hook.command);
            if !dup {
                out.push(hook);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matcher_wildcard_and_absent_match_everything() {
        assert!(matcher_matches(None, Some("Bash")));
        assert!(matcher_matches(Some("*"), Some("Bash")));
        assert!(matcher_matches(Some("   "), Some("Bash")));
    }

    #[test]
    fn matcher_exact_and_pipe_list() {
        // 只含 [A-Za-z0-9_- ,|] 的 matcher 走精确匹配
        assert!(matcher_matches(Some("Bash"), Some("Bash")));
        assert!(!matcher_matches(Some("Bash"), Some("NotebookEdit")));
        assert!(matcher_matches(Some("Edit|Write"), Some("Write")));
        assert!(matcher_matches(Some("Edit, Write"), Some("Edit")));
        // 带空格不影响
        assert!(matcher_matches(
            Some("code-reviewer"),
            Some("code-reviewer")
        ));
    }

    #[test]
    fn matcher_regex_path_is_unanchored() {
        // 含 `.` 等其他字符 → 走非锚定正则，任意位置子串命中。
        // 这意味着 `Edit.*` 同时匹配 Edit 和 NotebookEdit —— 忠实复刻 CC。
        assert!(matcher_matches(Some("Edit.*"), Some("NotebookEdit")));
        assert!(matcher_matches(Some("^Edit$"), Some("Edit")));
        assert!(!matcher_matches(Some("^Edit$"), Some("NotebookEdit")));
        assert!(matcher_matches(
            Some("mcp__memory__.*"),
            Some("mcp__memory__add")
        ));
    }

    #[test]
    fn matcher_without_tool_name_only_passes_wildcards() {
        // SessionStart 这类事件没有 tool_name，带 matcher 的 hook 不能因此误放行
        assert!(!matcher_matches(Some("Bash"), None));
        assert!(matcher_matches(None, None));
    }

    #[test]
    fn invalid_regex_fails_closed() {
        // 编译失败时选择不放行：宁可漏跑一条 hook，也不能让它拦下所有工具
        assert!(!matcher_matches(Some("[unclosed"), Some("Bash")));
    }

    fn write_settings(dir: &Path, body: &str) {
        // 顺序要紧：先建好 .star 再让任何 Storage::new 走一遍向上查找。
        // `find_nearest_existing_star_dir` 有进程内缓存，若在 .star 还不存在时先
        // 查一次，会把「没找到」按目录缓存住，之后这次测试再读就串到 home 下去了。
        let star = dir.join(".star");
        std::fs::create_dir_all(&star).unwrap();
        std::fs::write(star.join("settings.json"), body).unwrap();
    }

    #[test]
    fn parses_nested_cc_format() {
        let dir = std::env::temp_dir().join(format!("starcode-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_settings(
            &dir,
            r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[
                 {"type":"command","command":"./guard.sh","timeout":5}]}]}}"#,
        );

        let hooks = load_settings_hooks(&dir);
        let bash_hook = hooks
            .iter()
            .find(|h| h.matcher.as_deref() == Some("Bash"))
            .expect("Bash matcher 的 hook 应被解析出来");
        assert_eq!(bash_hook.event, ManagedHookEvent::PreToolUse);
        assert_eq!(bash_hook.command, "./guard.sh");
        assert_eq!(bash_hook.timeout_secs, 5);
        assert!(
            bash_hook.source.is_some(),
            "source 应记录来源文件供 /hooks 展示"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn args_are_appended_to_command() {
        let dir = std::env::temp_dir().join(format!("starcode-hooks-args-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_settings(
            &dir,
            r#"{"hooks":{"PostToolUse":[{"hooks":[
                 {"type":"command","command":"node","args":["fmt.js","--fix"]}]}]}}"#,
        );
        let hooks = load_settings_hooks(&dir);
        assert!(hooks.iter().any(|h| h.command == "node fmt.js --fix"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_cc_event_is_skipped_without_failing_the_file() {
        let dir = std::env::temp_dir().join(format!("starcode-hooks-ev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 用一个**真的**不存在的��件名 —— 断言的是「未知事件被跳过，
        // 且不连累同文件的合法条目」这件事本身，不是「哪个事件名不存在」。
        //
        // 早先这里写的是 `PermissionRequest`，那是在 PermissionRequest 还不
        // 支持的年代写的；事件补齐之后它变成了合法事件，断言就翻了。
        write_settings(
            &dir,
            r#"{"hooks":{"NoSuchEvent":[{"hooks":[
                 {"type":"command","command":"nope.sh"}]}],
                     "PreToolUse":[{"hooks":[{"type":"command","command":"ok.sh"}]}]}}"#,
        );
        let hooks = load_settings_hooks(&dir);
        assert!(
            !hooks.iter().any(|h| h.command == "nope.sh"),
            "未知事件名应被跳过"
        );
        assert!(
            hooks.iter().any(|h| h.command == "ok.sh"),
            "同文件里的合法事件不该被坏条目连累"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 补齐的 Claude Code 兼容事件必须真的能被 settings.json 认出来 ——
    /// 它们之前只存在于一个不参与编译的文件里，看着像支持，实际不是。
    #[test]
    fn claude_code_compatible_events_are_loadable() {
        let dir = std::env::temp_dir().join(format!(
            "starcode-hooks-cc-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_settings(
            &dir,
            r#"{"hooks":{
                 "PermissionRequest":[{"hooks":[{"type":"command","command":"perm.sh"}]}],
                 "PermissionDenied":[{"hooks":[{"type":"command","command":"denied.sh"}]}],
                 "PostToolUseFailure":[{"hooks":[{"type":"command","command":"fail.sh"}]}],
                 "StopFailure":[{"hooks":[{"type":"command","command":"stopfail.sh"}]}],
                 "SubagentStart":[{"hooks":[{"type":"command","command":"substart.sh"}]}],
                 "PostCompact":[{"hooks":[{"type":"command","command":"compact.sh"}]}]
               }}"#,
        );
        let hooks = load_settings_hooks(&dir);
        for command in [
            "perm.sh",
            "denied.sh",
            "fail.sh",
            "stopfail.sh",
            "substart.sh",
            "compact.sh",
        ] {
            assert!(
                hooks.iter().any(|h| h.command == command),
                "事件 {command} 没被加载：{hooks:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_command_handler_is_skipped() {
        let dir = std::env::temp_dir().join(format!("starcode-hooks-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_settings(
            &dir,
            r#"{"hooks":{"PreToolUse":[{"hooks":[
                 {"type":"http","url":"https://example.com/hook"}]}]}}"#,
        );
        assert!(load_settings_hooks(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_hooks_key_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("starcode-hooks-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_settings(&dir, r#"{"model":"gpt-4"}"#);
        assert!(load_settings_hooks(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
