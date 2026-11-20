use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;

pub async fn help(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    // /help keys — 打开键位快捷键弹窗（与 F1 相同）
    if args.first().map(|s| s.as_str()) == Some("keys") {
        ctx.state.show_help = true;
        return Ok(());
    }

    // /help <command> — 打印该命令的说明与子命令；裸 /help 仍打开命令面板。
    if !args.is_empty() {
        let unknown_command = || {
            format!(
                "Unknown command: `{}`.\n\nRun `/help` to browse all commands.",
                args.join(" ")
            )
        };
        let builtin = crate::commands::system::find_command_path(&args).or_else(|| {
            // 单个名称也遵守提交端的唯一前缀解析；多段未知路径不猜测。
            if args.len() == 1 && !args[0].is_empty() && !args[0].chars().any(char::is_whitespace) {
                crate::commands::system::parse_command(&format!("/{}", args[0]))
                    .and_then(|parsed| crate::commands::system::find_command_path(&parsed.path))
            } else {
                None
            }
        });
        let message = match builtin {
            Some((command, path)) => crate::commands::system::render_command_help(command, &path),
            // 内置占名规则与补全共享，避免展示实际无法进入插件分发的用法。
            None if args.len() == 1
                && crate::commands::system::can_dispatch_plugin_command(&args[0]) =>
            {
                match std::env::current_dir() {
                    Ok(cwd) => {
                        match crate::core::plugins::resolve_plugin_command(&cwd, &args[0]).await {
                            Ok(Some(command)) => crate::core::i18n::t1("cmd.help.plugin_details")
                                .replace("{name}", &command.name)
                                .replace("{source}", &command.source)
                                .replace("{description}", &command.description),
                            Ok(None) => unknown_command(),
                            Err(error) => crate::core::i18n::t1("cmd.help.plugin_error")
                                .replace("{error}", &error),
                        }
                    }
                    Err(error) => crate::core::i18n::t1("cmd.help.plugin_error")
                        .replace("{error}", &error.to_string()),
                }
            }
            None => unknown_command(),
        };
        ctx.state
            .chat_history
            .push(ChatEntry::assistant(message).with_streaming(false));
        return Ok(());
    }

    // 默认：打开可搜索的命令面板（分类 + 描述），替代 150 行文字倾倒
    ctx.state.show_help = false;
    ctx.state.palette_history.clear();
    ctx.state
        .open_palette(crate::ui::state::palette::PaletteMode::Help);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn bare_help_opens_the_command_palette() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        help(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        assert_eq!(
            state.palette_mode,
            crate::ui::state::palette::PaletteMode::Help
        );
        assert!(
            state.chat_history.iter().all(|entry| entry.is_welcome),
            "bare /help must not append a text dump"
        );
    }

    #[tokio::test]
    async fn help_with_a_command_prints_details_instead_of_opening_the_palette() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        help(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["config".to_string()],
        )
        .await
        .unwrap();

        assert_ne!(
            state.palette_mode,
            crate::ui::state::palette::PaletteMode::Help
        );
        let reply = state
            .chat_history
            .last()
            .expect("command details")
            .content
            .clone();
        assert!(reply.contains("# /config"));
        assert!(reply.contains("**Subcommands:**"));
        assert!(reply.contains("/config set"));
    }

    #[tokio::test]
    async fn help_with_an_alias_resolves_to_the_canonical_command() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        help(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["compact".to_string()],
        )
        .await
        .unwrap();

        let reply = &state.chat_history.last().expect("command details").content;
        assert!(reply.contains("# /compress"), "got: {reply}");
        assert!(reply.contains("`/compact`"));
    }

    #[tokio::test]
    async fn help_with_an_unknown_command_explains_the_mistake() {
        let _fixture = PluginHelpFixture::new();
        let reply = plugin_help_reply(&["definitely-not-a-command"]).await;
        assert!(reply.contains("Unknown command"));
        assert!(reply.contains("/help"));
    }

    struct PluginHelpFixture {
        project: crate::ui::test_support::TestProject,
        _global_config: tempfile::TempDir,
        previous_config: Option<std::ffi::OsString>,
    }

    impl PluginHelpFixture {
        fn new() -> Self {
            let project = crate::ui::test_support::TestProject::new();
            let global_config = tempfile::tempdir().expect("create isolated global plugin state");
            let previous_config = std::env::var_os("STAR_CONFIG_DIR");
            // 这些测试按项目约定串行运行，独立全局配置避免读取用户已安装插件。
            std::env::set_var("STAR_CONFIG_DIR", global_config.path());
            Self {
                project,
                _global_config: global_config,
                previous_config,
            }
        }

        fn assert_not_executed(&self, plugin: &str) {
            let marker = "plugin-help-executed.marker";
            assert!(!self.project.path().join(marker).exists());
            assert!(!crate::core::plugins::plugins_dir(self.project.path())
                .join(plugin)
                .join(marker)
                .exists());
        }

        async fn register(&self, plugin: &str, command: &str, enabled: bool) {
            use crate::core::plugins::{self, PluginEntry};
            let root = plugins::plugins_dir(self.project.path()).join(plugin);
            std::fs::create_dir_all(root.join(".star-plugin")).expect("create plugin metadata");
            std::fs::write(
                root.join(".star-plugin/plugin.json"),
                serde_json::to_string(&serde_json::json!({
                    "name": plugin,
                    "commands": [{
                        "name": command,
                        "description": "Inspect local fixture metadata",
                        "command": "echo plugin-help-executed > plugin-help-executed.marker"
                    }]
                }))
                .expect("serialize plugin metadata"),
            )
            .expect("write plugin metadata");
            let mut manifest = plugins::load_manifest(self.project.path())
                .await
                .expect("load isolated plugin manifest");
            manifest.plugins.push(PluginEntry {
                name: plugin.to_string(),
                source: "local-help-fixture".to_string(),
                install_type: "local".to_string(),
                installed_at: 0,
                enabled,
                scope: plugins::SCOPE_PROJECT.to_string(),
            });
            plugins::save_manifest(self.project.path(), &manifest)
                .await
                .expect("save isolated plugin manifest");
        }
    }

    impl Drop for PluginHelpFixture {
        fn drop(&mut self) {
            match self.previous_config.take() {
                Some(value) => std::env::set_var("STAR_CONFIG_DIR", value),
                None => std::env::remove_var("STAR_CONFIG_DIR"),
            }
        }
    }

    async fn plugin_help_reply(args: &[&str]) -> String {
        let mut state = crate::ui::state::ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);
        help(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            args.iter().map(|arg| (*arg).to_string()).collect(),
        )
        .await
        .expect("help is a read-only command");
        assert!(
            agent_rx.try_recv().is_err(),
            "help must not invoke the agent"
        );
        state
            .chat_history
            .last()
            .expect("help reply")
            .content
            .clone()
    }

    #[tokio::test]
    async fn help_for_plugin_command_shows_metadata_without_execution() {
        let fixture = PluginHelpFixture::new();
        fixture
            .register("fixture-plugin", "fixture-check", true)
            .await;
        let reply = plugin_help_reply(&["fixture-check"]).await;
        assert!(reply.contains("# /fixture-check"), "{reply}");
        assert!(reply.contains("Inspect local fixture metadata"), "{reply}");
        assert!(reply.contains("plugin:fixture-plugin"), "{reply}");
        assert!(
            !reply.contains("echo plugin-help-executed >"),
            "must not expose the shell payload: {reply}"
        );
        let marker = "plugin-help-executed.marker";
        assert!(
            !fixture.project.path().join(marker).exists(),
            "help must not execute the payload in the project directory"
        );
        assert!(
            !crate::core::plugins::plugins_dir(fixture.project.path())
                .join("fixture-plugin")
                .join(marker)
                .exists(),
            "help must not execute the payload in the plugin directory"
        );
    }

    #[tokio::test]
    async fn help_for_plugin_command_refreshes_after_disable_and_removal() {
        use crate::core::plugins;
        let fixture = PluginHelpFixture::new();
        fixture
            .register("fixture-plugin", "fixture-check", true)
            .await;
        assert!(plugin_help_reply(&["fixture-check"])
            .await
            .contains("# /fixture-check"));

        let mut manifest = plugins::load_manifest(fixture.project.path())
            .await
            .unwrap();
        manifest.plugins[0].enabled = false;
        plugins::save_manifest(fixture.project.path(), &manifest)
            .await
            .unwrap();
        assert!(plugin_help_reply(&["fixture-check"])
            .await
            .contains("Unknown command"));

        manifest.plugins[0].enabled = true;
        plugins::save_manifest(fixture.project.path(), &manifest)
            .await
            .unwrap();
        assert!(plugin_help_reply(&["fixture-check"])
            .await
            .contains("# /fixture-check"));
        manifest.plugins.clear();
        plugins::save_manifest(fixture.project.path(), &manifest)
            .await
            .unwrap();
        assert!(plugin_help_reply(&["fixture-check"])
            .await
            .contains("Unknown command"));
    }

    #[tokio::test]
    async fn help_for_plugin_command_reports_ambiguous_sources() {
        let fixture = PluginHelpFixture::new();
        fixture
            .register("fixture-first", "fixture-check", true)
            .await;
        fixture
            .register("fixture-second", "fixture-check", true)
            .await;
        let reply = plugin_help_reply(&["fixture-check"]).await;
        assert!(reply.contains("ambiguous"), "{reply}");
        assert!(reply.contains("plugin:fixture-first"), "{reply}");
        assert!(reply.contains("plugin:fixture-second"), "{reply}");
    }

    #[tokio::test]
    async fn help_for_plugin_command_keeps_builtin_aliases_and_paths_authoritative() {
        let fixture = PluginHelpFixture::new();
        fixture.register("fixture-plugin", "compact", true).await;
        let reply = plugin_help_reply(&["compact"]).await;
        assert!(reply.contains("# /compress"), "{reply}");
        assert!(!reply.contains("plugin:fixture-plugin"), "{reply}");

        fixture
            .register("fixture-other", "fixture-check", true)
            .await;
        let reply = plugin_help_reply(&["fixture-check", "unexpected"]).await;
        assert!(reply.contains("Unknown command"), "{reply}");
        assert!(!reply.contains("Inspect local fixture metadata"), "{reply}");
    }

    #[tokio::test]
    async fn help_for_plugin_command_respects_builtin_prefix_reservations() {
        let fixture = PluginHelpFixture::new();
        fixture.register("fixture-prefix", "comp", true).await;
        let parsed = crate::commands::system::parse_command("/comp").expect("唯一内置前缀");
        assert_eq!(parsed.path, vec!["compress"]);
        let reply = plugin_help_reply(&["comp"]).await;
        assert!(reply.contains("# /compress"), "{reply}");
        assert!(!reply.contains("plugin:fixture-prefix"), "{reply}");
        fixture.assert_not_executed("fixture-prefix");
    }

    #[tokio::test]
    async fn help_for_plugin_command_excludes_legacy_dispatch_reservations() {
        let fixture = PluginHelpFixture::new();
        fixture.register("fixture-legacy", "git-status", true).await;
        let reply = plugin_help_reply(&["git-status"]).await;
        assert!(!reply.contains("plugin:fixture-legacy"), "{reply}");
        assert!(!reply.contains("Inspect local fixture metadata"), "{reply}");
        assert!(reply.contains("Unknown command"), "{reply}");
        fixture.assert_not_executed("fixture-legacy");
    }

    #[tokio::test]
    async fn help_for_plugin_command_rejects_whitespace_inside_one_name() {
        let _fixture = PluginHelpFixture::new();
        for name in [
            "comp extra",
            "config set",
            "comp\textra",
            " comp",
            "comp ",
            "",
        ] {
            let reply = plugin_help_reply(&[name]).await;
            assert!(reply.contains("Unknown command"), "{name:?}: {reply}");
        }
    }

    #[tokio::test]
    async fn help_for_plugin_command_allows_ambiguous_builtin_prefix() {
        let fixture = PluginHelpFixture::new();
        fixture.register("fixture-ambiguous", "re", true).await;
        assert!(crate::commands::system::parse_command("/re").is_none());
        let reply = plugin_help_reply(&["re"]).await;
        assert!(reply.contains("# /re"), "{reply}");
        assert!(reply.contains("plugin:fixture-ambiguous"), "{reply}");
        fixture.assert_not_executed("fixture-ambiguous");
    }

    #[tokio::test]
    async fn help_for_plugin_command_excludes_disabled_runtime_commands() {
        let fixture = PluginHelpFixture::new();
        fixture
            .register("fixture-plugin", "fixture-check", true)
            .await;
        let manifest = crate::core::plugins::plugins_dir(fixture.project.path())
            .join("fixture-plugin/.star-plugin/plugin.json");
        let mut content: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&manifest).expect("read plugin runtime manifest"),
        )
        .expect("parse plugin runtime manifest");
        content["commands"][0]["enabled"] = serde_json::Value::Bool(false);
        std::fs::write(&manifest, serde_json::to_string(&content).unwrap()).unwrap();
        let reply = plugin_help_reply(&["fixture-check"]).await;
        assert!(reply.contains("Unknown command"), "{reply}");
        assert!(!reply.contains("Inspect local fixture metadata"), "{reply}");
    }

    #[tokio::test]
    async fn help_for_plugin_command_reports_registry_errors_without_hiding_builtin_help() {
        let fixture = PluginHelpFixture::new();
        let path = crate::core::plugins::manifest_path(fixture.project.path());
        std::fs::create_dir_all(path.parent().expect("manifest directory")).unwrap();
        std::fs::write(path, "{broken").unwrap();
        let builtin = plugin_help_reply(&["compact"]).await;
        assert!(builtin.contains("# /compress"), "{builtin}");
        let reply = plugin_help_reply(&["fixture-check"]).await;
        assert!(
            reply.contains("Unable to load plugin command help"),
            "{reply}"
        );
        assert!(reply.contains("failed to parse"), "{reply}");
        assert!(!reply.contains("Unknown command"), "{reply}");
    }

    #[tokio::test]
    async fn clear_remembers_the_requested_name_only_until_confirmation() {
        let mut state = crate::ui::state::ChatState::new();
        state
            .chat_history
            .push(crate::types::ChatEntry::user("keep this"));
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        clear(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["my".to_string(), "checkpoint".to_string()],
        )
        .await
        .unwrap();

        assert!(state.show_clear_confirmation);
        assert_eq!(state.pending_clear_name.as_deref(), Some("my checkpoint"));

        state.show_clear_confirmation = false;
        clear(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();
        assert_eq!(state.pending_clear_name, None);
        assert!(state.show_clear_confirmation);
    }

    #[tokio::test]
    async fn clear_on_an_empty_session_does_not_open_confirmation() {
        let mut state = crate::ui::state::ChatState::new();
        let (agent_tx, _agent_rx) = mpsc::channel(1);
        clear(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            vec!["ignored".to_string()],
        )
        .await
        .unwrap();

        assert!(!state.show_clear_confirmation);
        assert_eq!(state.pending_clear_name, None);
    }

    /// `/status` 是用户被告知"这次启动到底读了哪些配置"的唯一地方，
    /// 段标题与四个 settings 源一个都不能少。
    #[tokio::test]
    async fn status_reports_state_and_every_setting_source() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        state.current_model = "claude-sonnet-5-5".to_string();
        state.approval_mode = crate::types::ApprovalMode::Plan;
        state.git_branch = Some("dev-m".to_string());
        state.sandbox_enabled = true;
        let (agent_tx, _agent_rx) = mpsc::channel(1);

        status(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let report = &state.chat_history.last().expect("status report").content;
        assert!(report.starts_with("# 应用状态"), "{report}");
        assert!(report.contains("**模型:** claude-sonnet-5-5"), "{report}");
        assert!(report.contains("**Git:** dev-m"), "{report}");
        assert!(report.contains("Plan"), "{report}");
        assert!(report.contains("**沙箱:** 启用"), "{report}");

        // 四个 settings 文件必须逐个列出（存在与否是机器状态，不做断言）
        assert_eq!(
            report
                .lines()
                .filter(|line| line.starts_with("- ["))
                .count(),
            4,
            "{report}"
        );
        assert!(report.contains("user-settings.json"), "{report}");
        assert!(report.contains("settings.local.json"), "{report}");
        assert!(report.contains("生效的权限规则: allow="), "{report}");
    }

    /// 没有 token 数据、没有 git 仓库时也要给出明确说法，而不是留空白。
    #[tokio::test]
    async fn status_degrades_gracefully_without_usage_or_git() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        state.current_model.clear();
        state.token_usage = None;
        state.git_branch = None;
        let (agent_tx, _agent_rx) = mpsc::channel(1);

        status(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let report = &state.chat_history.last().expect("status report").content;
        assert!(report.contains("**模型:** 未设置"), "{report}");
        assert!(report.contains("**Token:** 暂无数据"), "{report}");
        assert!(report.contains("**Git:** 无仓库"), "{report}");
    }

    #[tokio::test]
    async fn exit_only_flags_the_session_for_shutdown() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        let (agent_tx, mut agent_rx) = mpsc::channel(1);

        exit(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(state.should_exit);
        // 保存/关闭必须由 run_app 编排；命令自己发 Shutdown 会让 SessionEnd hook
        // 和原生会话保存各跑两遍。
        assert!(
            agent_rx.try_recv().is_err(),
            "exit must not talk to the worker directly"
        );
    }

    #[tokio::test]
    async fn about_reports_the_crate_version() {
        let mut state = crate::ui::state::ChatState::test_fixture();
        let (agent_tx, _agent_rx) = mpsc::channel(1);

        about(
            CommandContext {
                state: &mut state,
                agent_tx: &agent_tx,
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let reply = &state.chat_history.last().expect("about").content;
        assert!(reply.contains(env!("CARGO_PKG_VERSION")), "{reply}");
    }
}

pub async fn clear(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    // Show confirmation before clearing (like Claude Code)
    if ctx.state.chat_history.iter().any(|e| !e.is_welcome) {
        ctx.state.pending_clear_name = if args.is_empty() {
            None
        } else {
            Some(args.join(" "))
        };
        ctx.state.show_clear_confirmation = true;
    } else {
        ctx.state.pending_clear_name = None;
        // Nothing to clear
        ctx.state.current_status_line =
            Some(crate::core::i18n::t1("ui.status.nothing_to_clear").to_string());
    }
    Ok(())
}

pub async fn exit(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    // 由 run_app 统一编排保存、worker shutdown 与 SessionEnd hook；这里不能直接
    // process::exit，否则 worker 持有的原生会话上下文会丢失且 hooks 可能重复执行。
    ctx.state.should_exit = true;
    Ok(())
}

pub async fn status(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let mut lines: Vec<String> = Vec::new();
    lines.push("# 应用状态\n".to_string());

    // 模型
    let model = if ctx.state.current_model.is_empty() {
        "未设置"
    } else {
        &ctx.state.current_model
    };
    lines.push(format!("**模型:** {}", model));

    // Token 用量
    if let Some(usage) = &ctx.state.token_usage {
        let provider_id = crate::ui::utils::status::current_provider_id(ctx.state);
        let ctx_win = crate::agent::model_catalog::get_cached_context_window(
            provider_id.as_deref(),
            &ctx.state.current_model,
        )
        .map(|c| c as usize)
        .or_else(|| {
            std::env::var("STAR_CONTEXT_WINDOW")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(200_000);
        let pct = if ctx_win > 0 {
            (usage.prompt_tokens as f64 / ctx_win as f64) * 100.0
        } else {
            0.0
        };
        lines.push(format!(
            "**Token:** prompt={}, completion={}, total={} (上下文 {:.1}%)",
            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens, pct
        ));
    } else {
        lines.push("**Token:** 暂无数据".to_string());
    }

    // 对话轮次
    let msg_count = ctx.state.chat_history.len();
    lines.push(format!("**对话消息:** {} 条", msg_count));

    // 处理状态
    if ctx.state.is_processing {
        lines.push("**状态:** 处理中".to_string());
    } else if ctx.state.is_streaming {
        lines.push("**状态:** 流式输出中".to_string());
    } else {
        lines.push("**状态:** 空闲".to_string());
    }

    // Git
    if let Some(branch) = &ctx.state.git_branch {
        let status_str = ctx.state.git_status.as_deref().unwrap_or("未知");
        lines.push(format!("**Git:** {} ({})", branch, status_str));
    } else {
        lines.push("**Git:** 无仓库".to_string());
    }

    // 审批模式
    let mode = ctx.state.approval_mode.label_zh();
    lines.push(format!("**审批模式:** {}", mode));

    // 沙箱
    lines.push(format!(
        "**沙箱:** {}",
        if ctx.state.sandbox_enabled {
            "启用"
        } else {
            "禁用"
        }
    ));

    // 语言
    lines.push(format!(
        "**界面语言:** {}",
        crate::core::i18n::current_language().as_code()
    ));

    // 提供商会话数
    if let Ok(store) = crate::core::config::provider_store::ProviderStore::new()
        .load()
        .await
    {
        lines.push(format!("**已配置供应商:** {} 个", store.providers.len()));
    }

    lines.extend(setting_sources_section(ctx.state));

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(lines.join("\n")).with_streaming(false));
    Ok(())
}

/// `/status` 的 **Setting sources** 段（对标 CC `/status` 的同名一行）。
///
/// 存在的理由不是"列个路径好看"，而是 StarCode 的全局层**有两个** settings 文件名：
/// `user-settings.json` 管 model/provider/凭据，`settings.json` 管 `permissions`
/// 与 `hooks`（见 `Storage::global_settings_path` 的函数文档）。用户把 allow/deny
/// 写进其中一个文件却被另一个的同名 key 盖住时，没有任何界面能看出这回事 ——
/// `/status` 是唯一能一眼看出"我这次到底读了哪些文件、哪些存在"的地方。
fn setting_sources_section(state: &crate::ui::state::ChatState) -> Vec<String> {
    use crate::core::config::storage::Storage;

    let cwd = std::env::current_dir().unwrap_or_default();
    let storage = Storage::new(cwd.clone());

    // (标签, 路径, 这一层负责什么)
    let sources: Vec<(&str, std::path::PathBuf, &str)> = vec![
        (
            "user",
            Storage::global_settings_path(),
            "permissions / hooks",
        ),
        (
            "user",
            Storage::global_star_dir().join("user-settings.json"),
            "defaultModel / providers / apiKey",
        ),
        (
            "project",
            storage.workspace_settings_path(),
            "committable project settings",
        ),
        (
            "local",
            storage.star_dir().join("settings.local.json"),
            "personal overrides (gitignored)",
        ),
    ];

    let mut out = vec![String::new(), "**Setting sources**".to_string()];
    let mut any = false;
    for (scope, path, owns) in sources {
        let exists = path.exists();
        any |= exists;
        out.push(format!(
            "- [{}] `{}`{} — {}",
            scope,
            path.display(),
            if exists { "" } else { " (missing)" },
            owns
        ));
    }

    // 权限规则的来源与数量：与上面的文件清单同一套 `SettingsPermissions`
    let perms = crate::core::policy::SettingsPermissions::from_project(&cwd);
    out.push(format!(
        "- 生效的权限规则: allow={} ask={} deny={}",
        perms.allow.len(),
        perms.ask.len(),
        perms.deny.len()
    ));
    if perms.default_mode.is_some() {
        out.push(format!(
            "- settings `defaultMode`: `{}`（本次启动：{}）",
            perms.default_mode.as_deref().unwrap_or(""),
            state
                .current_status_line
                .as_deref()
                .unwrap_or("以状态栏为准"),
        ));
    }
    if !any {
        out.push("- 提示：没有任何配置文件存在，所有行为走内置默认值。".to_string());
    }

    out
}

pub async fn about(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let version = env!("CARGO_PKG_VERSION");
    let content = format!("Starcode CLI v{}\n", version);

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}
