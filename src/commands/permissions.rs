use crate::commands::execution::{CommandContext, CommandResult};
use crate::core::permission_rules::{PermissionAction, PermissionRule, RuleSource};
use crate::types::ApprovalMode;

pub async fn run(ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.is_empty() {
        let msg = format!(
            "当前审批模式: `{}`\n\n可用命令:\n- `/permissions default`\n- `/permissions plan`\n- `/permissions yolo`\n- `/permissions acceptEdits` (映射为 default)\n- `/permissions bypassPermissions` (映射为 yolo)\n- `/permissions list` - 查看所有规则\n- `/permissions add <tool> <action> [path]` - 添加规则\n- `/permissions remove <id>` - 删除规则\n- `/permissions deny-log` - 查看拒绝日志\n- `/permissions clear-log` - 清空拒绝日志",
            mode_label(&ctx.state.approval_mode)
        );

        ctx.state
            .chat_history
            .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        return Ok(());
    }

    let raw = args[0].trim().to_lowercase();
    match raw.as_str() {
        "default" | "acceptedits" => {
            ctx.state.approval_mode = ApprovalMode::Default;
            let _ = ctx
                .agent_tx
                .send(crate::runtime::messages::AgentRequest::SetApprovalMode(
                    ApprovalMode::Default,
                ))
                .await;
            let msg = "✅ 审批模式已切换为 `default`".to_string();
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "plan" | "readonly" => {
            ctx.state.approval_mode = ApprovalMode::Plan;
            let _ = ctx
                .agent_tx
                .send(crate::runtime::messages::AgentRequest::SetApprovalMode(
                    ApprovalMode::Plan,
                ))
                .await;
            let msg = "✅ 审批模式已切换为 `plan`".to_string();
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "yolo" | "bypasspermissions" => {
            ctx.state.approval_mode = ApprovalMode::Yolo;
            let _ = ctx
                .agent_tx
                .send(crate::runtime::messages::AgentRequest::SetApprovalMode(
                    ApprovalMode::Yolo,
                ))
                .await;
            let msg = "✅ 审批模式已切换为 `yolo`".to_string();
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "list" => {
            let mut msg = String::from("📋 权限规则列表:\n\n");
            let rules = ctx.state.permission_rules.get_rules();
            if rules.is_empty() {
                msg.push_str("暂无自定义规则。");
            } else {
                for rule in rules {
                    let action_str = match &rule.action {
                        PermissionAction::Allow => "✅ 允许".to_string(),
                        PermissionAction::Deny => "❌ 拒绝".to_string(),
                        PermissionAction::Ask(msg) => format!("❓ 询问: {}", msg),
                    };
                    let path_str = rule
                        .path_pattern
                        .as_ref()
                        .map(|p| format!(" 路径: `{}`", p))
                        .unwrap_or_default();
                    msg.push_str(&format!(
                        "- [{}] `{}` {}{}\n",
                        rule.id, rule.tool_pattern, action_str, path_str
                    ));
                }
            }
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "add" => {
            if args.len() < 3 {
                return Err("用法: /permissions add <tool> <action> [path]\n例如: /permissions add bash allow src/**".to_string());
            }
            let tool = args[1].clone();
            let action = match args[2].as_str() {
                "allow" => PermissionAction::Allow,
                "deny" => PermissionAction::Deny,
                other => PermissionAction::Ask(other.to_string()),
            };
            let path = if args.len() > 3 {
                Some(args[3].clone())
            } else {
                None
            };
            let id = format!("rule_{}", ctx.state.permission_rules.get_rules().len());
            let rule = PermissionRule {
                id: id.clone(),
                name: tool.clone(),
                tool_pattern: tool.clone(),
                path_pattern: path.clone(),
                action,
                priority: 0,
                source: RuleSource::Project,
            };
            ctx.state.permission_rules.add_rule(rule);
            let msg = format!("✅ 已添加规则 `{}`: {}{}", id, tool, path.map(|p| format!(" 路径: {}", p)).unwrap_or_default());
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "remove" => {
            if args.len() < 2 {
                return Err("用法: /permissions remove <id>".to_string());
            }
            let id = &args[1];
            ctx.state.permission_rules.remove_rule(id);
            let msg = format!("✅ 已删除规则 `{}`", id);
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "deny-log" => {
            let log = ctx.state.permission_rules.get_deny_log();
            let records = log.get_records();
            let mut msg = format!("🚫 拒绝日志 ({} 条记录):\n\n", records.len());
            if records.is_empty() {
                msg.push_str("暂无拒绝记录。");
            } else {
                for record in records.iter().take(20) {
                    let time = chrono::DateTime::from_timestamp(record.timestamp, 0)
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    msg.push_str(&format!(
                        "- [{}] {} - {} ({})\n",
                        time, record.tool, record.reason, record.args
                    ));
                }
                if records.len() > 20 {
                    msg.push_str(&format!("\n... 还有 {} 条记录", records.len() - 20));
                }
            }
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        "clear-log" => {
            ctx.state.permission_rules.get_deny_log().clear();
            let msg = "✅ 拒绝日志已清空".to_string();
            ctx.state
                .chat_history
                .push(crate::types::ChatEntry::assistant(msg).with_streaming(false));
        }
        _ => {
            return Err(format!(
                "未知权限命令: {} (可用: default|plan|yolo|list|add|remove|deny-log|clear-log)",
                args[0]
            ));
        }
    }

    Ok(())
}

fn mode_label(mode: &ApprovalMode) -> &'static str {
    match mode {
        ApprovalMode::Default => "default",
        ApprovalMode::Plan => "plan",
        ApprovalMode::Yolo => "yolo",
    }
}
