use crate::commands::execution::{CommandContext, CommandResult};
use crate::types::ChatEntry;

pub async fn help(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    // /help keys — 打开键位快捷键弹窗（与 F1 相同）
    if args.first().map(|s| s.as_str()) == Some("keys") {
        ctx.state.show_help = true;
        return Ok(());
    }

    // 默认：打开可搜索的命令面板（分类 + 描述），替代 150 行文字倾倒
    ctx.state.show_help = false;
    ctx.state.show_palette = true;
    ctx.state.palette_mode = crate::ui::state::palette::PaletteMode::Help;
    ctx.state.palette_items = crate::ui::components::palette::get_help_palette_items();
    ctx.state.palette_filter.clear();
    ctx.state.selected_palette_index = 0;
    ctx.state.palette_history.clear();
    Ok(())
}

pub async fn clear(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    // Show confirmation before clearing (like Claude Code)
    if ctx.state.chat_history.iter().any(|e| !e.is_welcome) {
        ctx.state.show_clear_confirmation = true;
    } else {
        // Nothing to clear
        ctx.state.current_status_line = Some(
            crate::core::i18n::t("ui.status.nothing_to_clear", "没有可清除的内容", "Nothing to clear").to_string()
        );
    }
    Ok(())
}

pub async fn exit(_ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    if let Ok(cwd) = std::env::current_dir() {
        let _ = crate::core::hooks::runner::run_hooks(
            &cwd,
            crate::core::hooks::store::ManagedHookEvent::SessionEnd,
            &crate::core::hooks::runner::HookRunContext {
                user_message: String::new(),
                status: "session_end".to_string(),
                tool_name: None,
                tool_arguments: None,
                tool_success: None,
                stop_reason: Some("user_exit".to_string()),
                stop_hook_active: false,
            },
        )
        .await;
    }
    std::process::exit(0);
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
        let ctx_win = crate::agent::model_catalog::get_cached_context_window(&ctx.state.current_model)
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
        let status_str = ctx.state
            .git_status
            .as_deref()
            .unwrap_or("未知");
        lines.push(format!("**Git:** {} ({})", branch, status_str));
    } else {
        lines.push("**Git:** 无仓库".to_string());
    }

    // 审批模式
    let mode = match ctx.state.approval_mode {
        crate::types::ApprovalMode::Default => "默认",
        crate::types::ApprovalMode::Plan => "Plan (计划模式)",
        crate::types::ApprovalMode::Yolo => "YOLO (自动审批)",
    };
    lines.push(format!("**审批模式:** {}", mode));

    // 沙箱
    lines.push(format!(
        "**沙箱:** {}",
        if ctx.state.sandbox_enabled { "启用" } else { "禁用" }
    ));

    // 语言
    lines.push(format!(
        "**界面语言:** {}",
        crate::core::i18n::current_language().as_code()
    ));

    // 提供商会话数
    if let Ok(store) = crate::core::config::provider_store::ProviderStore::new().load().await {
        lines.push(format!(
            "**已配置供应商:** {} 个",
            store.providers.len()
        ));
    }

    ctx.state.chat_history.push(
        ChatEntry::assistant(lines.join("\n")).with_streaming(false),
    );
    Ok(())
}

pub async fn about(ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let version = env!("CARGO_PKG_VERSION");
    let content = format!("Starcode CLI v{}\n", version);

    ctx.state
        .chat_history
        .push(ChatEntry::assistant(content).with_streaming(false));
    Ok(())
}
