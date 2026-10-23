/// `/permissions` —— 审批模式 + Claude Code 口径的权限规则（allow / ask / deny）。
///
/// 规则的解析和判定在 [`crate::core::policy::settings_rules`]；这里只做两件事：
/// 把磁盘上的规则渲染出来，以及把用户加的规则写进项目级 `.star/settings.local.json`。
///
/// 写完必须发 [`AgentRequest::ReloadPermissions`]：PolicyEngine 只活在 MessageBus 里，
/// UI 侧碰不到它，不重载就等于改了一份没人读的文件。
///
/// `list` 直接读盘而不是向 MessageBus 要快照 —— 省一条 StreamMessage 往返，
/// 而且刚写完就能读到结果。
use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::policy::settings_rules::{self, PermissionRuleSpec, RuleVerdict};
use crate::core::policy::SettingsPermissions;
use crate::runtime::messages::AgentRequest;
use crate::types::ApprovalMode;
use std::path::{Path, PathBuf};

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    if args.is_empty() {
        let msg = render_overview(&cwd, &ctx.state.approval_mode);
        reply(ctx.state, msg);
        return Ok(());
    }

    let verb = args[0].trim().to_lowercase();
    let rest = &args[1..];

    // 规则动词先判，模式别名后判：`approval_mode_from_str` 也认 "ask"，
    // 而 `/permissions ask <rule>` 明显是在加规则。
    match verb.as_str() {
        "list" | "ls" => {
            let msg = render_overview(&cwd, &ctx.state.approval_mode);
            reply(ctx.state, msg);
        }
        "allow" | "ask" | "deny" => {
            let bucket = settings_rules::bucket_from_str(&verb).unwrap_or(RuleVerdict::Ask);
            let rule = join_rule(rest).ok_or_else(|| usage_for_bucket(&verb))?;
            let msg = add_rule(&cwd, bucket, &rule)?;
            let _ = ctx.agent_tx.send(AgentRequest::ReloadPermissions).await;
            reply(ctx.state, msg);
        }
        "remove" | "rm" => {
            let rule = join_rule(rest).ok_or_else(|| {
                "Usage: /permissions remove <rule>   e.g. /permissions remove Bash(cargo test:*)"
                    .to_string()
            })?;
            let msg = remove_rule(&cwd, &rule)?;
            let _ = ctx.agent_tx.send(AgentRequest::ReloadPermissions).await;
            reply(ctx.state, msg);
        }
        // 旧写法 `add <tool> <action> [specifier]`，翻成一条规则字符串。
        "add" => {
            if rest.len() < 2 {
                return Err(concat!(
                    "Usage: /permissions add <tool> <allow|ask|deny> [specifier]\n",
                    "Or use the direct form: /permissions allow Bash(cargo test:*)"
                )
                .to_string());
            }
            let bucket = settings_rules::bucket_from_str(&rest[1]).ok_or_else(|| {
                format!(
                    "Unknown action `{}` (expected allow, ask or deny).",
                    rest[1]
                )
            })?;
            let tool = normalize_tool_name(&rest[0]);
            let rule = if rest.len() > 2 {
                format!("{}({})", tool, rest[2..].join(" "))
            } else {
                tool
            };
            let msg = add_rule(&cwd, bucket, &rule)?;
            let _ = ctx.agent_tx.send(AgentRequest::ReloadPermissions).await;
            reply(ctx.state, msg);
        }
        "reload" => {
            let perms = SettingsPermissions::from_project(&cwd);
            let _ = ctx.agent_tx.send(AgentRequest::ReloadPermissions).await;
            reply(
                ctx.state,
                format!(
                    "Reloaded {} permission rule(s) from disk.",
                    perms.rule_count()
                ),
            );
        }
        "deny-log" => {
            let msg = render_deny_log(&ctx.state.permission_rules);
            reply(ctx.state, msg);
        }
        "clear-log" => {
            ctx.state.permission_rules.get_deny_log().clear();
            reply(ctx.state, "Deny log cleared.".to_string());
        }
        other => {
            let Some(mode) = mode_from_verb(other) else {
                return Err(format!(
                    "Unknown /permissions argument `{}`.\nRules: allow | ask | deny | remove | list\nModes: default | acceptEdits | plan | yolo | bypassPermissions\nLog: deny-log | clear-log",
                    args[0]
                ));
            };
            ctx.state.approval_mode = mode.clone();
            let _ = ctx
                .agent_tx
                .send(AgentRequest::SetApprovalMode(mode.clone()))
                .await;
            reply(
                ctx.state,
                format!("Approval mode set to `{}`.", mode_label(&mode)),
            );
        }
    }

    Ok(())
}

fn reply(state: &mut crate::ui::state::store::ChatState, msg: String) {
    state
        .chat_history
        .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
}

/// 当前生效的模式 + 规则 + 来源文件。规则是 settings 各层的并集，
/// 所以逐个文件再解析一遍才能标出每条规则是哪来的。
fn render_overview(cwd: &Path, mode: &ApprovalMode) -> String {
    render_overview_with(cwd, mode, SettingsPermissions::from_project(cwd))
}

/// `merged` 由调用方给：单测不该被开发机 `~/.star/settings.json` 里的规则左右。
fn render_overview_with(cwd: &Path, mode: &ApprovalMode, merged: SettingsPermissions) -> String {
    let mut out = format!("**Approval mode:** `{}`\n", mode_label(mode));

    if let Some(default_mode) = merged.default_mode.as_ref() {
        out.push_str(&format!("**settings `defaultMode`:** `{}`\n", default_mode));
    }

    out.push('\n');
    if merged.is_empty() {
        out.push_str("No permission rules configured.\n");
    } else {
        for bucket in [RuleVerdict::Deny, RuleVerdict::Ask, RuleVerdict::Allow] {
            let rules = bucket_of(&merged, bucket);
            if rules.is_empty() {
                continue;
            }
            out.push_str(&format!("**{}** ({})\n", bucket_title(bucket), rules.len()));
            for rule in rules {
                out.push_str(&format!("- `{}`\n", rule.raw));
            }
            out.push('\n');
        }
        out.push_str("Precedence: deny > ask > allow. Deny also overrides yolo mode.\n");
    }

    if !merged.additional_directories.is_empty() {
        out.push_str("\n**additionalDirectories**\n");
        for dir in &merged.additional_directories {
            out.push_str(&format!("- `{}`\n", dir));
        }
    }

    out.push_str("\n**Sources** (later files win on `defaultMode`)\n");
    for path in SettingsPermissions::candidate_files(cwd) {
        if !path.exists() {
            continue;
        }
        let per_file = SettingsPermissions::from_paths(std::slice::from_ref(&path));
        out.push_str(&format!(
            "- `{}` — {} rule(s)\n",
            path.display(),
            per_file.rule_count()
        ));
    }
    out.push_str(&format!(
        "\nRules added here land in `{}`.\n",
        settings_rules::local_settings_path(cwd).display()
    ));
    out.push_str(concat!(
        "\nUsage: `/permissions allow|ask|deny <rule>`, `/permissions remove <rule>`\n",
        "Examples: `Bash(cargo test:*)`, `Read(~/.cargo/config.toml)`, ",
        "`Edit(src/**)`, `WebFetch(domain:docs.rs)`, `mcp__github`\n"
    ));
    out
}

fn render_deny_log(engine: &crate::core::permission_rules::PermissionRuleEngine) -> String {
    let records = engine.get_deny_log().get_records();
    let mut out = format!("**Deny log** ({} record(s))\n\n", records.len());
    if records.is_empty() {
        out.push_str("No denied tool calls recorded.");
        return out;
    }
    for record in records.iter().take(20) {
        let time = chrono::DateTime::from_timestamp(record.timestamp, 0)
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        out.push_str(&format!(
            "- [{}] {} — {} ({})\n",
            time, record.tool, record.reason, record.args
        ));
    }
    if records.len() > 20 {
        out.push_str(&format!("\n... {} more record(s)", records.len() - 20));
    }
    out
}

/// 加规则。同一条规则会先从本地文件的其它桶里摘掉 —— 用户写
/// `/permissions deny X` 就是要覆盖之前的 `allow X`，留着两份只会让
/// deny > allow 的优先级去替他解释。
fn add_rule(cwd: &Path, bucket: RuleVerdict, rule: &str) -> Result<String, String> {
    let spec = PermissionRuleSpec::parse(rule).ok_or_else(|| {
        format!(
            "Invalid rule `{}`. Expected `Tool` or `Tool(specifier)`, e.g. `Bash(cargo test:*)`.",
            rule
        )
    })?;

    let local = SettingsPermissions::from_paths(&[settings_rules::local_settings_path(cwd)]);
    let held_by: Vec<RuleVerdict> = [RuleVerdict::Deny, RuleVerdict::Ask, RuleVerdict::Allow]
        .into_iter()
        .filter(|b| bucket_of(&local, *b).iter().any(|r| r.raw == spec.raw))
        .collect();

    if held_by == [bucket] {
        return Ok(format!(
            "`{}` is already in `permissions.{}`.",
            spec.raw,
            settings_rules::bucket_key(bucket)
        ));
    }

    let moved: Vec<RuleVerdict> = settings_rules::remove_local_rule(cwd, &spec.raw)?
        .into_iter()
        .filter(|b| *b != bucket)
        .collect();
    let path = settings_rules::add_local_rule(cwd, bucket, &spec.raw)?
        .unwrap_or_else(|| settings_rules::local_settings_path(cwd));

    let mut out = format!(
        "Added `{}` to `permissions.{}` in `{}`.",
        spec.raw,
        settings_rules::bucket_key(bucket),
        path.display()
    );
    if !moved.is_empty() {
        out.push_str(&format!(
            "\nRemoved it from `{}` to keep one verdict per rule.",
            moved
                .iter()
                .map(|b| format!("permissions.{}", settings_rules::bucket_key(*b)))
                .collect::<Vec<_>>()
                .join("`, `")
        ));
    }
    out.push_str("\nRules reloaded — this takes effect on the next tool call.");
    Ok(out)
}

/// 删规则。只动 `.star/settings.local.json` —— 写在项目 `settings.json` 或全局层的
/// 规则可能是团队共享的，命令不替用户改，只告诉他在哪。
fn remove_rule(cwd: &Path, rule: &str) -> Result<String, String> {
    let raw = PermissionRuleSpec::parse(rule)
        .map(|spec| spec.raw)
        .unwrap_or_else(|| rule.trim().to_string());

    let removed = settings_rules::remove_local_rule(cwd, &raw)?;
    if !removed.is_empty() {
        return Ok(format!(
            "Removed `{}` from `{}` in `{}`.",
            raw,
            removed
                .iter()
                .map(|b| format!("permissions.{}", settings_rules::bucket_key(*b)))
                .collect::<Vec<_>>()
                .join("`, `"),
            settings_rules::local_settings_path(cwd).display()
        ));
    }

    let elsewhere = locate_rule(cwd, &raw);
    if elsewhere.is_empty() {
        return Ok(format!("`{}` is not in any permission rule list.", raw));
    }
    let mut out = format!(
        "`{}` is not in `settings.local.json`. It comes from:\n",
        raw
    );
    for (path, bucket) in elsewhere {
        out.push_str(&format!(
            "- `{}` → `permissions.{}`\n",
            path.display(),
            settings_rules::bucket_key(bucket)
        ));
    }
    out.push_str("Edit that file directly, then run `/permissions reload`.");
    Ok(out)
}

/// 逐个候选文件找一条规则的出处，给 remove 的"不在本地层"提示用。
fn locate_rule(cwd: &Path, raw: &str) -> Vec<(PathBuf, RuleVerdict)> {
    let mut hits = Vec::new();
    for path in SettingsPermissions::candidate_files(cwd) {
        if !path.exists() {
            continue;
        }
        let per_file = SettingsPermissions::from_paths(std::slice::from_ref(&path));
        for bucket in [RuleVerdict::Deny, RuleVerdict::Ask, RuleVerdict::Allow] {
            if bucket_of(&per_file, bucket).iter().any(|r| r.raw == raw) {
                hits.push((path.clone(), bucket));
            }
        }
    }
    hits
}

fn bucket_of(perms: &SettingsPermissions, bucket: RuleVerdict) -> &[PermissionRuleSpec] {
    match bucket {
        RuleVerdict::Allow => &perms.allow,
        RuleVerdict::Ask => &perms.ask,
        RuleVerdict::Deny => &perms.deny,
    }
}

fn bucket_title(bucket: RuleVerdict) -> &'static str {
    match bucket {
        RuleVerdict::Allow => "Allow — run without asking",
        RuleVerdict::Ask => "Ask — always confirm",
        RuleVerdict::Deny => "Deny — never run",
    }
}

/// 参数是按空白切开的，`Bash(cargo test:*)` 会散成两片，拼回去即可。
fn join_rule(rest: &[String]) -> Option<String> {
    let joined = rest.join(" ").trim().to_string();
    let trimmed = joined
        .trim_matches(|c| c == '"' || c == '\'')
        .trim()
        .to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn usage_for_bucket(verb: &str) -> String {
    format!(
        "Usage: /permissions {} <rule>\nExamples: /permissions {} Bash(cargo test:*)   |   /permissions {} Read(./.env)",
        verb, verb, verb
    )
}

/// 用户按 Claude Code 的口径打 `bash` / `read`，落盘时统一成注册表里的写法。
fn normalize_tool_name(raw: &str) -> String {
    let raw = raw.trim();
    let canonical = crate::core::tools::constants::canonical_tool_name(raw);
    if canonical != raw {
        return canonical;
    }
    crate::core::tools::constants::all_builtin_tool_names()
        .into_iter()
        .find(|name| name.eq_ignore_ascii_case(raw))
        .map(|name| name.to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// 模式别名复用 `settings_rules` 那一份（和 `defaultMode`、`--permission-mode` 同源），
/// 这里只把 policy 侧的枚举翻成 UI 侧的。
fn mode_from_verb(verb: &str) -> Option<ApprovalMode> {
    use crate::core::policy::types::ApprovalMode as PolicyMode;
    match settings_rules::approval_mode_from_str(verb)? {
        PolicyMode::Default => Some(ApprovalMode::Default),
        PolicyMode::AcceptEdits => Some(ApprovalMode::AcceptEdits),
        PolicyMode::Plan => Some(ApprovalMode::Plan),
        PolicyMode::Yolo => Some(ApprovalMode::Yolo),
    }
}

fn mode_label(mode: &ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::Default => "default",
        ApprovalMode::AcceptEdits => "acceptEdits",
        ApprovalMode::Plan => "plan",
        ApprovalMode::Yolo => "yolo",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 规则文件是 `.star/settings.local.json`；单测只碰临时目录，绝不落到仓库里。
    fn perms_with_raw(allow: &[&str], ask: &[&str], deny: &[&str]) -> SettingsPermissions {
        let mut perms = SettingsPermissions::default();
        perms.allow = allow
            .iter()
            .filter_map(|raw| PermissionRuleSpec::parse(raw))
            .collect();
        perms.ask = ask
            .iter()
            .filter_map(|raw| PermissionRuleSpec::parse(raw))
            .collect();
        perms.deny = deny
            .iter()
            .filter_map(|raw| PermissionRuleSpec::parse(raw))
            .collect();
        perms
    }

    fn local_rule_buckets(cwd: &Path) -> Vec<(String, String)> {
        let local = SettingsPermissions::from_paths(&[settings_rules::local_settings_path(cwd)]);
        let mut out = Vec::new();
        for (bucket, rules) in [
            ("allow", &local.allow),
            ("ask", &local.ask),
            ("deny", &local.deny),
        ] {
            for rule in rules {
                out.push((bucket.to_string(), rule.raw.clone()));
            }
        }
        out
    }

    #[test]
    fn overview_states_the_mode_and_where_rules_land() {
        let dir = tempfile::tempdir().unwrap();
        let out = render_overview_with(
            dir.path(),
            &ApprovalMode::AcceptEdits,
            SettingsPermissions::default(),
        );

        assert!(out.contains("**Approval mode:** `acceptEdits`"), "{out}");
        assert!(out.contains("No permission rules configured."), "{out}");
        assert!(
            out.contains("Rules added here land in"),
            "the overview must say where writes go:\n{out}"
        );
        assert!(
            out.contains("/permissions allow|ask|deny <rule>"),
            "the overview is also the command's mini-help:\n{out}"
        );
    }

    /// 展示顺序即优先级：deny 在最前面，用户从上往下读就是判定顺序。
    #[test]
    fn overview_groups_rules_in_precedence_order() {
        let dir = tempfile::tempdir().unwrap();
        let merged = perms_with_raw(&["Read(src/**)"], &["Bash(git push:*)"], &["Edit(.env)"]);

        let out = render_overview_with(dir.path(), &ApprovalMode::Default, merged);
        let deny_at = out.find("**Deny").expect("deny group");
        let ask_at = out.find("**Ask").expect("ask group");
        let allow_at = out.find("**Allow").expect("allow group");
        assert!(deny_at < ask_at && ask_at < allow_at, "{out}");
        assert!(out.contains("Precedence: deny > ask > allow"), "{out}");
    }

    #[test]
    fn overview_only_reports_settings_default_mode_when_one_is_set() {
        let dir = tempfile::tempdir().unwrap();

        let without = render_overview_with(
            dir.path(),
            &ApprovalMode::Default,
            SettingsPermissions::default(),
        );
        assert!(!without.contains("settings `defaultMode`"), "{without}");

        let mut merged = SettingsPermissions::default();
        merged.default_mode = Some("plan".to_string());
        let with = render_overview_with(dir.path(), &ApprovalMode::Plan, merged);
        assert!(
            with.contains("**settings `defaultMode`:** `plan`"),
            "{with}"
        );
    }

    #[test]
    fn join_rule_reassembles_split_specifiers() {
        assert_eq!(
            join_rule(&["Bash(cargo".to_string(), "test:*)".to_string()]),
            Some("Bash(cargo test:*)".to_string())
        );
        assert_eq!(
            join_rule(&["\"Read(./.env)\"".to_string()]),
            Some("Read(./.env)".to_string()),
            "shell quotes are the user's, not part of the rule"
        );
        assert_eq!(join_rule(&[]), None);
        assert_eq!(join_rule(&["   ".to_string()]), None);
    }

    #[test]
    fn adding_a_rule_writes_the_local_layer() {
        let dir = tempfile::tempdir().unwrap();

        let msg = add_rule(dir.path(), RuleVerdict::Allow, "Bash(cargo test:*)").unwrap();
        assert!(msg.contains("Added `Bash(cargo test:*)`"), "{msg}");
        assert!(msg.contains("permissions.allow"), "{msg}");
        assert_eq!(
            local_rule_buckets(dir.path()),
            vec![("allow".to_string(), "Bash(cargo test:*)".to_string())]
        );
    }

    /// 同一规则只能有一个裁决：用户先说 allow、后说 deny，就是在改口。
    #[test]
    fn re_adding_a_rule_moves_it_between_buckets() {
        let dir = tempfile::tempdir().unwrap();
        add_rule(dir.path(), RuleVerdict::Allow, "Edit(src/**)").unwrap();

        let msg = add_rule(dir.path(), RuleVerdict::Deny, "Edit(src/**)").unwrap();
        assert!(msg.contains("Removed it from `permissions.allow`"), "{msg}");
        assert_eq!(
            local_rule_buckets(dir.path()),
            vec![("deny".to_string(), "Edit(src/**)".to_string())],
            "the old allow entry must not survive"
        );

        let again = add_rule(dir.path(), RuleVerdict::Deny, "Edit(src/**)").unwrap();
        assert!(
            again.contains("is already in `permissions.deny`"),
            "{again}"
        );
        assert_eq!(local_rule_buckets(dir.path()).len(), 1);
    }

    #[test]
    fn adding_an_invalid_rule_is_rejected_before_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let err = add_rule(dir.path(), RuleVerdict::Allow, "   ").unwrap_err();
        assert!(err.contains("Invalid rule"), "{err}");
        assert!(!settings_rules::local_settings_path(dir.path()).exists());
    }

    #[test]
    fn removing_a_missing_rule_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let msg = remove_rule(dir.path(), "Bash(rm:*)").unwrap();
        assert!(msg.contains("is not in any permission rule list"), "{msg}");
    }

    #[test]
    fn removing_a_local_rule_reports_the_bucket() {
        let dir = tempfile::tempdir().unwrap();
        add_rule(dir.path(), RuleVerdict::Ask, "Bash(git push:*)").unwrap();

        let msg = remove_rule(dir.path(), "Bash(git push:*)").unwrap();
        assert!(msg.contains("Removed `Bash(git push:*)`"), "{msg}");
        assert!(msg.contains("permissions.ask"), "{msg}");
        assert!(local_rule_buckets(dir.path()).is_empty());
    }

    /// 共享层的规则不替用户改，但要指明确切出处，否则用户会以为命令没生效。
    #[test]
    fn removing_a_rule_from_a_shared_layer_points_at_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let star = dir.path().join(".star");
        std::fs::create_dir_all(&star).unwrap();
        std::fs::write(
            star.join("settings.json"),
            r#"{"permissions":{"deny":["Bash(rm:*)"]}}"#,
        )
        .unwrap();

        let msg = remove_rule(dir.path(), "Bash(rm:*)").unwrap();
        assert!(msg.contains("is not in `settings.local.json`"), "{msg}");
        assert!(msg.contains("settings.json"), "{msg}");
        assert!(msg.contains("permissions.deny"), "{msg}");
        assert!(msg.contains("/permissions reload"), "{msg}");
    }

    #[test]
    fn tool_names_are_normalized_to_registered_spelling() {
        assert_eq!(normalize_tool_name("bash"), "Bash");
        assert_eq!(normalize_tool_name("Read"), "Read");
        assert_eq!(normalize_tool_name("todowrite"), "TodoWrite");
        // 对标 CC：内置工具名即唯一名，不再维护 `view_file` → `Read` 这类别名表，
        // 所以旧写法会原样落盘（规则匹配不到任何调用，用户能立刻看出来）。
        assert_eq!(normalize_tool_name("view_file"), "view_file");
        // 未知名字原样保留：MCP 的 `mcp__server` 之类不在内建表里。
        assert_eq!(normalize_tool_name("mcp__github"), "mcp__github");
    }

    #[test]
    fn mode_verbs_accept_claude_code_spellings() {
        assert_eq!(mode_from_verb("default"), Some(ApprovalMode::Default));
        assert_eq!(mode_from_verb("ask"), Some(ApprovalMode::Default));
        assert_eq!(
            mode_from_verb("acceptEdits"),
            Some(ApprovalMode::AcceptEdits)
        );
        assert_eq!(mode_from_verb("readonly"), Some(ApprovalMode::Plan));
        assert_eq!(
            mode_from_verb("bypassPermissions"),
            Some(ApprovalMode::Yolo)
        );
        assert_eq!(mode_from_verb("nonsense"), None);
    }

    #[test]
    fn mode_labels_round_trip_through_the_verbs() {
        for mode in [
            ApprovalMode::Default,
            ApprovalMode::AcceptEdits,
            ApprovalMode::Plan,
            ApprovalMode::Yolo,
        ] {
            let label = mode_label(&mode);
            assert_eq!(
                mode_from_verb(label),
                Some(mode.clone()),
                "`{label}` must parse back to the same mode"
            );
        }
    }

    #[tokio::test]
    async fn unknown_arguments_are_errors_not_silent_no_ops() {
        use crate::commands::execution::CommandContext;

        let mut state = crate::ui::state::ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);
        let before = state.chat_history.len();

        let err = run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["frobnicate".to_string()],
        )
        .await
        .unwrap_err();

        assert!(err.contains("Unknown /permissions argument"), "{err}");
        assert_eq!(
            state.chat_history.len(),
            before,
            "a rejected argument must not leave a half-written message"
        );
    }

    #[tokio::test]
    async fn an_empty_rule_after_a_bucket_is_a_usage_error() {
        use crate::commands::execution::CommandContext;

        let mut state = crate::ui::state::ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);

        let err = run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["allow".to_string()],
        )
        .await
        .unwrap_err();
        assert!(err.contains("Usage: /permissions allow <rule>"), "{err}");
    }

    /// 旧写法 `/permissions add <tool> <action> [spec]` 必须继续可用，
    /// 并且把 `bash` 之类的写法归一到注册名。
    #[tokio::test]
    async fn the_legacy_add_form_still_works() {
        use crate::commands::execution::CommandContext;

        // 这里只验证参数校验失败的那一侧（不落盘）：动作名非法。
        let mut state = crate::ui::state::ChatState::test_fixture();
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(1);

        let err = run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["add".to_string(), "bash".to_string(), "maybe".to_string()],
        )
        .await
        .unwrap_err();
        assert!(err.contains("Unknown action `maybe`"), "{err}");

        let err = run(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["add".to_string(), "bash".to_string()],
        )
        .await
        .unwrap_err();
        assert!(err.contains("Usage: /permissions add"), "{err}");
    }

    #[test]
    fn deny_log_rendering_caps_at_twenty_records() {
        let engine = crate::core::permission_rules::PermissionRuleEngine::new();
        for i in 0..25 {
            engine.get_deny_log().record(
                "Bash",
                &serde_json::json!({ "command": format!("cmd-{i}") }),
                "test",
                None,
            );
        }

        let out = render_deny_log(&engine);
        assert!(out.contains("**Deny log** (25 record(s))"), "{out}");
        assert!(out.contains("cmd-0"), "{out}");
        assert!(out.contains("cmd-19"), "{out}");
        assert!(!out.contains("cmd-20"), "{out}");
        assert!(out.contains("... 5 more record(s)"), "{out}");
    }

    #[test]
    fn the_deny_log_is_empty_by_default() {
        let engine = crate::core::permission_rules::PermissionRuleEngine::new();
        let out = render_deny_log(&engine);
        assert!(out.contains("No denied tool calls recorded."), "{out}");
    }
}
