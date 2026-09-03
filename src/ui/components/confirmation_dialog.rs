use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::core::i18n;

/// Permission dialog colors (matching Claude Code dark theme)
const PERMISSION_COLOR: Color = Color::Rgb(177, 185, 249); // Light blue-purple
const SUGGESTION_COLOR: Color = Color::Rgb(177, 185, 249); // Same as permission
const SUCCESS_COLOR: Color = Color::Rgb(78, 186, 101); // Bright green
const ERROR_COLOR: Color = Color::Rgb(255, 107, 128); // Bright red
const INACTIVE_COLOR: Color = Color::Rgb(153, 153, 153); // Light gray
const SUBTLE_COLOR: Color = Color::Rgb(80, 80, 80); // Dark gray

fn risk_label(risk: &crate::types::RiskLevel) -> String {
    let shape = match risk {
        crate::types::RiskLevel::Safe => "✓",
        crate::types::RiskLevel::Low => "○",
        crate::types::RiskLevel::Medium => "△",
        crate::types::RiskLevel::High => "▲",
        crate::types::RiskLevel::Critical => "⚠",
    };
    let text = match risk {
        crate::types::RiskLevel::Safe => i18n::t("ui.confirm.risk.safe", "安全", "Safe"),
        crate::types::RiskLevel::Low => i18n::t("ui.confirm.risk.low", "低风险", "Low"),
        crate::types::RiskLevel::Medium => i18n::t("ui.confirm.risk.medium", "中风险", "Medium"),
        crate::types::RiskLevel::High => i18n::t("ui.confirm.risk.high", "高风险", "High"),
        crate::types::RiskLevel::Critical => {
            i18n::t("ui.confirm.risk.critical", "严重风险", "Critical")
        }
    };
    format!("{} {}", shape, text)
}

fn risk_color(risk: &crate::types::RiskLevel) -> Color {
    match risk {
        crate::types::RiskLevel::Safe => Color::Green,
        crate::types::RiskLevel::Low => Color::Cyan,
        crate::types::RiskLevel::Medium => Color::Yellow,
        crate::types::RiskLevel::High => Color::LightRed,
        crate::types::RiskLevel::Critical => Color::Red,
    }
}

/// 非 Shell 类确认卡的风险分级。
///
/// ShellCommand 自带 `estimated_risk`（`tool_gate::estimate_bash_risk`），本函数只管其余类型：
/// 它们原先只有一行布尔警告「可能修改文件或执行命令」，改不改系统目录都是同一句话。
/// 落到系统路径的写/删按 `dangerous_patterns::is_system_path` 抬一级。
fn estimate_details_risk(confirmation: &crate::types::ToolConfirmation) -> crate::types::RiskLevel {
    use crate::core::auto_mode::dangerous_patterns::is_system_path;
    use crate::types::{ConfirmationDetails, RiskLevel};

    match &confirmation.details {
        ConfirmationDetails::DeleteFile { file_path } => {
            if is_system_path(file_path) {
                RiskLevel::Critical
            } else {
                RiskLevel::High
            }
        }
        ConfirmationDetails::EditFile { file_path, .. }
        | ConfirmationDetails::CreateFile { file_path, .. } => {
            if is_system_path(file_path) {
                RiskLevel::High
            } else {
                RiskLevel::Low
            }
        }
        ConfirmationDetails::NetworkRequest { .. } => RiskLevel::Medium,
        ConfirmationDetails::AskUserQuestion { .. } => RiskLevel::Safe,
        // Generic 覆盖 MCP / 插件等无结构信息的工具，只能听 is_dangerous
        ConfirmationDetails::Generic { .. } | ConfirmationDetails::ShellCommand { .. } => {
            if confirmation.is_dangerous {
                RiskLevel::Medium
            } else {
                RiskLevel::Low
            }
        }
    }
}

pub fn build_confirmation_card_block(
    confirmation: &crate::types::ToolConfirmation,
    wrap_width: usize,
    selected_choice: usize,
    show_explanation: bool,
    show_debug: bool,
) -> Vec<Line<'static>> {
    use crate::types::{ConfirmationDetails, ConfirmationType};

    // If already resolved, show outcome (Claude Code style: compact single line)
    if let Some(outcome) = &confirmation.outcome {
        let rejected = outcome.eq_ignore_ascii_case("Cancelled")
            || outcome.to_lowercase().contains("cancel")
            || outcome.to_lowercase().contains("rejected");
        let color = if rejected { ERROR_COLOR } else { SUCCESS_COLOR };
        let icon = if rejected { "✗" } else { "✓" };
        let detail = match &confirmation.details {
            ConfirmationDetails::EditFile { file_path, .. } => format_display_path(file_path),
            ConfirmationDetails::CreateFile { file_path, .. } => format_display_path(file_path),
            ConfirmationDetails::DeleteFile { file_path } => format_display_path(file_path),
            ConfirmationDetails::ShellCommand { command, .. } => {
                let cmd = command.trim();
                if cmd.is_empty() {
                    confirmation.tool_name.clone()
                } else {
                    format!("$ {}", truncate_str(cmd, 80))
                }
            }
            ConfirmationDetails::NetworkRequest { url, method } => {
                format!("{} {}", method, truncate_str(url, 60))
            }
            ConfirmationDetails::Generic { title, .. } => {
                if title.is_empty() {
                    confirmation.tool_name.clone()
                } else {
                    title.clone()
                }
            }
            ConfirmationDetails::AskUserQuestion {
                header, question, ..
            } => header.clone().unwrap_or_else(|| truncate_str(question, 40)),
        };
        return vec![Line::from(vec![
            Span::styled(
                format!("{} ", icon),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(outcome.clone(), Style::default().fg(color)),
            Span::styled(format!("  {}", detail), Style::default().fg(INACTIVE_COLOR)),
        ])];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── Title (bold, permission color) ──
    let title = match &confirmation.operation_type {
        ConfirmationType::EditFile => {
            i18n::t("ui.confirm.title.edit_file", "编辑文件", "Edit file")
        }
        ConfirmationType::ShellCommand => {
            i18n::t("ui.confirm.title.bash", "Bash 命令", "Bash command")
        }
        ConfirmationType::CreateFile => {
            i18n::t("ui.confirm.title.create_file", "新建文件", "Create file")
        }
        ConfirmationType::AskUserQuestion => "Question".to_string(), // rendered by build_ask_user_question_card
        _ => i18n::t("ui.confirm.title.tool_use", "工具调用", "Tool use"),
    };
    lines.push(Line::from(Span::styled(
        title,
        Style::default()
            .fg(PERMISSION_COLOR)
            .add_modifier(Modifier::BOLD),
    )));

    // ── Separator ──
    let sep = "─".repeat(wrap_width.saturating_sub(2));
    lines.push(Line::from(Span::styled(
        sep,
        Style::default().fg(SUBTLE_COLOR),
    )));

    // ── Content (command/file with description) ──
    match &confirmation.operation_type {
        ConfirmationType::EditFile => {
            if let ConfirmationDetails::EditFile {
                file_path,
                old_lines,
                new_lines,
                ..
            } = &confirmation.details
            {
                let added = new_lines.saturating_sub(*old_lines);
                let removed = old_lines.saturating_sub(*new_lines);
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(file_path.clone(), Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!(" (+{} -{})", added, removed),
                        Style::default().fg(INACTIVE_COLOR),
                    ),
                ]));
            }
        }
        ConfirmationType::ShellCommand => {
            if let ConfirmationDetails::ShellCommand {
                command,
                estimated_risk,
                diff_preview,
                ..
            } = &confirmation.details
            {
                let preview = truncate_str(command, wrap_width.saturating_sub(6) as usize);
                lines.push(Line::from(vec![
                    Span::styled("  $ ", Style::default().fg(SUCCESS_COLOR)),
                    Span::styled(preview, Style::default().fg(Color::White)),
                ]));
                lines.push(Line::from(vec![Span::styled(
                    format!(
                        "{}{}",
                        i18n::t("ui.confirm.label.risk", "  风险: ", "  Risk: "),
                        risk_label(estimated_risk)
                    ),
                    Style::default().fg(risk_color(estimated_risk)),
                )]));
                // Render diff preview if available
                if let Some(diff) = diff_preview {
                    if !diff.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!(
                                "  {}",
                                i18n::t("ui.confirm.label.diff", "变更预览:", "Diff preview:")
                            ),
                            Style::default().fg(INACTIVE_COLOR),
                        )));
                        for diff_line in diff.lines().take(20) {
                            let color = if diff_line.starts_with('+') {
                                Color::Green
                            } else if diff_line.starts_with('-') {
                                Color::Red
                            } else if diff_line.starts_with("@@") {
                                Color::Cyan
                            } else {
                                Color::Gray
                            };
                            lines.push(Line::from(Span::styled(
                                format!(
                                    "  {}",
                                    truncate_str(diff_line, wrap_width.saturating_sub(4) as usize)
                                ),
                                Style::default().fg(color),
                            )));
                        }
                    }
                }
            }
        }
        ConfirmationType::CreateFile => {
            if let ConfirmationDetails::CreateFile { file_path, .. } = &confirmation.details {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(file_path.clone(), Style::default().fg(Color::Yellow)),
                ]));
            }
        }
        ConfirmationType::AskUserQuestion => {
            // AskUserQuestion is rendered by build_ask_user_question_card() in tool_render.rs.
            // If we reach here, fall through gracefully.
        }
        _ => {
            if let ConfirmationDetails::Generic { title, prompt } = &confirmation.details {
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(truncate_str(title, 40), Style::default().fg(Color::White)),
                ]));
                if !prompt.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(
                            truncate_str(prompt, 60),
                            Style::default().fg(INACTIVE_COLOR),
                        ),
                    ]));
                }
            }
        }
    }

    // ── Risk line ──
    // ShellCommand 分支上面已经打印过分级风险，这里只补其余类型：原先它们无论改的是
    // 项目里的一个文件还是 /etc 下的系统文件，都只有同一句布尔警告。
    if !matches!(
        confirmation.operation_type,
        ConfirmationType::ShellCommand | ConfirmationType::AskUserQuestion
    ) {
        let risk = estimate_details_risk(confirmation);
        if confirmation.is_dangerous || !matches!(risk, crate::types::RiskLevel::Safe) {
            lines.push(Line::from(Span::styled(
                format!(
                    "{}{}",
                    i18n::t("ui.confirm.label.risk", "  风险: ", "  Risk: "),
                    risk_label(&risk)
                ),
                Style::default().fg(risk_color(&risk)).add_modifier(
                    if matches!(
                        risk,
                        crate::types::RiskLevel::High | crate::types::RiskLevel::Critical
                    ) {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    },
                ),
            )));
        }
    }

    // ── Blank line ──
    lines.push(Line::from(""));

    // ── Question ──
    lines.push(Line::from(Span::styled(
        i18n::t(
            "ui.confirm.question.proceed",
            "  是否继续？",
            "  Do you want to proceed?",
        ),
        Style::default().fg(Color::White),
    )));

    // ── Options (Claude Code style) ──
    let option = |choice: usize, label: &str| -> Line<'static> {
        let selected = selected_choice == choice;
        let indicator = if selected {
            Span::styled(
                "❯ ",
                Style::default()
                    .fg(SUGGESTION_COLOR)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("  ", Style::default())
        };
        let num = Span::styled(
            format!("{}. ", choice),
            Style::default().fg(if selected {
                SUGGESTION_COLOR
            } else {
                INACTIVE_COLOR
            }),
        );
        let text = Span::styled(
            label.to_string(),
            Style::default()
                .fg(if selected { Color::White } else { Color::Gray })
                .add_modifier(if selected {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        );
        Line::from(vec![indicator, num, text])
    };

    lines.push(option(
        1,
        &i18n::t("ui.confirm.option.once", "1. 允许一次", "1. Allow once"),
    ));
    lines.push(option(
        2,
        &i18n::t(
            "ui.confirm.option.session",
            "2. 本会话允许",
            "2. Allow for session",
        ),
    ));
    lines.push(option(
        3,
        &i18n::t("ui.confirm.option.always", "3. 永久允许", "3. Always allow"),
    ));
    lines.push(option(
        4,
        &i18n::t("ui.confirm.option.deny", "4. 拒绝", "4. Deny"),
    ));

    // ── Explanation section (Ctrl+E) ──
    if show_explanation {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            i18n::t(
                "ui.confirm.explain.title",
                "  为什么询问此权限？",
                "  Why is this permission needed?",
            ),
            Style::default()
                .fg(PERMISSION_COLOR)
                .add_modifier(Modifier::BOLD),
        )));
        let explanation = match &confirmation.details {
            ConfirmationDetails::EditFile { .. } | ConfirmationDetails::CreateFile { .. } => {
                i18n::t(
                    "ui.confirm.explain.edit",
                    "  该工具将修改你的文件系统。请检查上面的路径与变更是否符合预期。",
                    "  This tool will modify files on disk. Review the path and changes above.",
                )
            }
            ConfirmationDetails::DeleteFile { .. } => {
                i18n::t(
                    "ui.confirm.explain.delete",
                    "  该工具将删除文件，此操作不可恢复。",
                    "  This tool will delete a file, which cannot be undone.",
                )
            }
            ConfirmationDetails::ShellCommand { .. } => {
                i18n::t(
                    "ui.confirm.explain.bash",
                    "  该命令将在你的机器上执行，可能产生副作用。风险等级见上方标注。",
                    "  This command will run on your machine and may have side effects. See risk level above.",
                )
            }
            ConfirmationDetails::NetworkRequest { .. } => {
                i18n::t(
                    "ui.confirm.explain.network",
                    "  该工具将向外部服务发起网络请求。",
                    "  This tool will make a network request to an external service.",
                )
            }
            _ => {
                i18n::t(
                    "ui.confirm.explain.generic",
                    "  出于安全考虑，该操作需要你确认后才能执行。",
                    "  For safety, this action requires your approval before it runs.",
                )
            }
        };
        lines.push(Line::from(Span::styled(
            explanation.to_string(),
            Style::default().fg(INACTIVE_COLOR),
        )));
    }

    // ── Debug section (Ctrl+D): 工具入参 JSON ──
    if show_debug {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Debug (tool input)",
            Style::default()
                .fg(PERMISSION_COLOR)
                .add_modifier(Modifier::BOLD),
        )));
        let debug_json = match &confirmation.details {
            ConfirmationDetails::Generic { title, .. } => title.clone(),
            _ => format!(
                "{{\"tool\": \"{}\", \"operation\": \"{}\"}}",
                confirmation.tool_name,
                format!("{:?}", confirmation.operation_type)
                    .trim_start_matches("ConfirmationType::")
            ),
        };
        for l in wrap_text(&debug_json, wrap_width.saturating_sub(4)) {
            lines.push(Line::from(Span::styled(
                format!("  {}", l),
                Style::default().fg(SUBTLE_COLOR),
            )));
        }
    }

    // ── Bottom hint ──
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        i18n::t(
            "ui.confirm.hint.key_guide",
            "  Esc 拒绝 · 1-4 选择 · Enter 确认 · Ctrl+E 解释 / Ctrl+D 调试",
            "  Esc to reject · 1-4 to select · Enter to confirm · Ctrl+E explain · Ctrl+D debug",
        ),
        Style::default().fg(SUBTLE_COLOR),
    )));

    lines
}

/// Render an ask_user_question confirmation card with selectable options.
pub fn build_ask_user_question_card(
    confirmation: &crate::types::ToolConfirmation,
    wrap_width: usize,
    selected_choice: usize,     // 1-based focused option index
    selected_options: &[usize], // selected indices for multi-select
    other_input: &str,          // "Other" text input value
) -> Vec<Line<'static>> {
    use crate::types::ConfirmationDetails;

    // Already resolved — show outcome
    if let Some(outcome) = &confirmation.outcome {
        let rejected = outcome.eq_ignore_ascii_case("Cancelled")
            || outcome.to_lowercase().contains("cancel")
            || outcome.to_lowercase().contains("rejected");
        let color = if rejected { ERROR_COLOR } else { SUCCESS_COLOR };
        let icon = if rejected { "✗" } else { "✓" };
        let outcome_text = outcome.clone();
        return vec![Line::from(vec![
            Span::styled(
                format!("{} ", icon),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(outcome_text, Style::default().fg(color)),
        ])];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Extract owned data to satisfy 'static lifetime
    let (question_text, header_text, mode_label, options_data, multi_select_flag) =
        if let ConfirmationDetails::AskUserQuestion {
            question,
            header,
            options,
            multi_select,
        } = &confirmation.details
        {
            let h = header.clone().unwrap_or_else(|| "Question".to_string());
            let m = if *multi_select {
                " (多选)".to_string()
            } else {
                " (单选)".to_string()
            };
            let opts: Vec<(String, String)> = options
                .iter()
                .map(|o| (o.label.clone(), o.description.clone()))
                .collect();
            (question.clone(), h, m, opts, *multi_select)
        } else {
            return lines;
        };

    // ── Header ──
    lines.push(Line::from(vec![
        Span::styled(
            "? ",
            Style::default()
                .fg(SUGGESTION_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}{}", header_text, mode_label),
            Style::default()
                .fg(PERMISSION_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // ── Separator ──
    let sep = "─".repeat(wrap_width.saturating_sub(2));
    lines.push(Line::from(Span::styled(
        sep,
        Style::default().fg(SUBTLE_COLOR),
    )));

    // ── Question text (wrapped) ──
    let max_line = wrap_width.saturating_sub(4);
    for line_text in question_text.split('\n') {
        if line_text.is_empty() {
            lines.push(Line::from(Span::raw("")));
            continue;
        }
        for chunk in wrap_text(line_text, max_line) {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(chunk, Style::default().fg(Color::White)),
            ]));
        }
    }
    lines.push(Line::from(""));

    // ── Options ──
    let max_desc_width = wrap_width.saturating_sub(8);
    let other_idx = options_data.len(); // 0-based: options indices are 0..N-1, other is N
    let other_input_owned = other_input.to_string();

    for (i, (label, desc)) in options_data.iter().enumerate() {
        let idx = i + 1;
        let is_focused = selected_choice == i;
        let is_selected = multi_select_flag && selected_options.contains(&i);

        let pointer = if is_focused {
            Span::styled(
                "❯ ",
                Style::default()
                    .fg(SUGGESTION_COLOR)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("  ", Style::default())
        };

        let focus_style = if is_focused {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let label_str = label.clone();
        if multi_select_flag {
            let checkbox = if is_selected { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                pointer,
                Span::styled(format!("{}. {} ", idx, checkbox), focus_style),
                Span::styled(label_str, focus_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                pointer,
                Span::styled(format!("{}. ", idx), focus_style),
                Span::styled(label_str, focus_style),
            ]));
        }

        if !desc.is_empty() {
            let desc_style = if is_focused {
                Style::default().fg(INACTIVE_COLOR)
            } else {
                Style::default().fg(SUBTLE_COLOR)
            };
            let desc_str = desc.clone();
            for chunk in wrap_text(&desc_str, max_desc_width) {
                lines.push(Line::from(vec![
                    Span::styled("     ", Style::default()),
                    Span::styled(chunk, desc_style),
                ]));
            }
        }
    }

    // ── "Other" option ──
    let is_other_focused = selected_choice == other_idx;
    let pointer = if is_other_focused {
        Span::styled(
            "❯ ",
            Style::default()
                .fg(SUGGESTION_COLOR)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("  ", Style::default())
    };
    let other_style = if is_other_focused {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    lines.push(Line::from(""));
    let other_display = if other_input_owned.is_empty() {
        "(type to input)".to_string()
    } else {
        other_input_owned.clone()
    };
    lines.push(Line::from(vec![
        pointer,
        Span::styled(format!("{}. Other: ", other_idx), other_style),
        Span::styled(
            other_display,
            if other_input_owned.is_empty() {
                Style::default().fg(SUBTLE_COLOR)
            } else {
                Style::default().fg(Color::White)
            },
        ),
    ]));

    // ── Help ──
    lines.push(Line::from(""));
    let help = if multi_select_flag {
        crate::core::i18n::t(
            "ui.confirm.help_multiselect",
            "Enter=确认  Esc=取消  1-9=切换  Space=多选切换  Tab=其他输入",
            "Enter=Confirm  Esc=Cancel  1-9=Toggle  Space=Multi-select  Tab=Other input",
        )
    } else {
        crate::core::i18n::t(
            "ui.confirm.help_single",
            "Enter=确认  Esc=取消  1-9=选择  ↑↓=导航  Tab/选Other后Enter=输入自定义",
            "Enter=Confirm  Esc=Cancel  1-9=Select  ↑↓=Navigate  Tab/Other+Enter=Custom input",
        )
    };
    lines.push(Line::from(Span::styled(
        format!("  {}", help),
        Style::default().fg(SUBTLE_COLOR),
    )));

    lines
}

/// Wrap text to a max character width, breaking at word boundaries when possible.
fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            result.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() > max_width {
                result.push(current);
                current = word.to_string();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    result
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

fn format_display_path(path: &str) -> String {
    if path.is_empty() {
        return "(unknown)".to_string();
    }
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let home = dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if !cwd.is_empty() && path.starts_with(&cwd) {
        let rel = &path[cwd.len()..];
        return rel.strip_prefix('/').unwrap_or(rel).to_string();
    }
    if !home.is_empty() && path.starts_with(&home) {
        return format!("~{}", &path[home.len()..]);
    }
    path.to_string()
}

pub async fn build_confirmation_from_tool_call(
    tc: &crate::types::StarToolCall,
) -> crate::types::ToolConfirmation {
    let args_value: serde_json::Value =
        serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Null);
    let get_str = |keys: &[&str]| -> Option<String> {
        for k in keys {
            if let Some(v) = args_value.get(*k).and_then(|vv| vv.as_str()) {
                return Some(v.to_string());
            }
        }
        None
    };
    let get_u64 = |keys: &[&str]| -> Option<u64> {
        for k in keys {
            if let Some(v) = args_value.get(*k).and_then(|vv| vv.as_u64()) {
                return Some(v);
            }
        }
        None
    };
    let get_bool = |keys: &[&str]| -> Option<bool> {
        for k in keys {
            if let Some(v) = args_value.get(*k).and_then(|vv| vv.as_bool()) {
                return Some(v);
            }
        }
        None
    };

    match tc.function.name.as_str() {
        "exit_plan_mode" => {
            let plan = get_str(&["plan"]).unwrap_or_else(|| {
                i18n::t("ui.confirm.msg.empty_plan", "（空计划）", "(empty plan)")
            });
            let preview = if plan.chars().count() > 800 {
                format!("{}...", plan.chars().take(800).collect::<String>())
            } else {
                plan
            };
            crate::types::ToolConfirmation {
                tool_name: "exit_plan_mode".to_string(),
                operation_type: crate::types::ConfirmationType::ShellCommand,
                details: crate::types::ConfirmationDetails::ShellCommand {
                    command: preview,
                    working_dir: std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| ".".to_string()),
                    estimated_risk: crate::types::RiskLevel::Low,
                    diff_preview: None,
                },
                is_dangerous: false,
                outcome: None,
            }
        }
        "Edit" | "str_replace_editor" => {
            let path = get_str(&["path", "file_path", "target_file"])
                .unwrap_or_else(|| "unknown".to_string());
            let old_str = get_str(&["old_string", "old_str", "old", "oldString", "old_text"])
                .unwrap_or_default();
            let new_str = get_str(&["new_string", "new_str", "new", "newString", "new_text"])
                .unwrap_or_default();
            let replace_all = get_bool(&["replace_all", "ReplaceAll"]).unwrap_or(false);

            let diff = if path != "unknown" && !old_str.is_empty() {
                match tokio::fs::read_to_string(&path).await {
                    Ok(content) => {
                        let new_content = if replace_all {
                            content.replace(&old_str, &new_str)
                        } else {
                            content.replacen(&old_str, &new_str, 1)
                        };
                        let old_lines: Vec<&str> = content.lines().collect();
                        let new_lines: Vec<&str> = new_content.lines().collect();
                        let max_len = old_lines.len().max(new_lines.len());
                        let mut out = String::new();
                        out.push_str(&format!("--- a/{}\n", path));
                        out.push_str(&format!("+++ b/{}\n", path));
                        let mut shown = 0usize;
                        for i in 0..max_len {
                            let o = old_lines.get(i).copied().unwrap_or("");
                            let n = new_lines.get(i).copied().unwrap_or("");
                            if o != n {
                                out.push_str(&format!("@@ -{},{} +{},{} @@\n", i + 1, 1, i + 1, 1));
                                out.push_str(&format!("-{}\n", o));
                                out.push_str(&format!("+{}\n", n));
                                shown += 1;
                                if shown >= 20 {
                                    out.push_str(&i18n::t(
                                        "ui.confirm.msg.diff_truncated",
                                        "...（差异已截断）\n",
                                        "... (diff truncated)\n",
                                    ));
                                    break;
                                }
                            }
                        }
                        if shown == 0 {
                            out.push_str(&i18n::t(
                                "ui.confirm.msg.no_diff",
                                "（无差异或替换未匹配）\n",
                                "(no differences or replacement not matched)\n",
                            ));
                        }
                        out
                    }
                    Err(e) => {
                        let tpl = i18n::t(
                            "ui.confirm.msg.failed_read",
                            "（无法读取文件，无法生成差异: {0}）\n- {1}\n+ {2}",
                            "(failed to read file, cannot generate diff: {0})\n- {1}\n+ {2}",
                        );
                        tpl.replace("{0}", &e.to_string())
                            .replace("{1}", &old_str)
                            .replace("{2}", &new_str)
                    }
                }
            } else {
                let raw = tc.function.arguments.chars().take(800).collect::<String>();
                {
                    let tpl = i18n::t(
                        "ui.confirm.msg.missing_params",
                        "（无法生成差异: 缺少参数）\nargs={0}\n- {1}\n+ {2}",
                        "(unable to generate diff: missing parameters)\nargs={0}\n- {1}\n+ {2}",
                    );
                    tpl.replace("{0}", &raw)
                        .replace("{1}", &old_str)
                        .replace("{2}", &new_str)
                }
            };

            crate::types::ToolConfirmation {
                tool_name: tc.function.name.clone(),
                operation_type: crate::types::ConfirmationType::EditFile,
                details: crate::types::ConfirmationDetails::EditFile {
                    file_path: path,
                    diff,
                    old_lines: old_str.lines().count(),
                    new_lines: new_str.lines().count(),
                },
                is_dangerous: false,
                outcome: None,
            }
        }
        "smart_edit" => {
            let path = get_str(&["file_path", "path", "target_file"])
                .unwrap_or_else(|| "unknown".to_string());
            let old_str =
                get_str(&["old_string", "old_str", "old", "old_text"]).unwrap_or_default();
            let new_str =
                get_str(&["new_string", "new_str", "new", "new_text"]).unwrap_or_default();

            let diff = if path != "unknown" && !old_str.is_empty() {
                match tokio::fs::read_to_string(&path).await {
                    Ok(content) => {
                        let new_content = content.replacen(&old_str, &new_str, 1);
                        let old_lines: Vec<&str> = content.lines().collect();
                        let new_lines: Vec<&str> = new_content.lines().collect();
                        let max_len = old_lines.len().max(new_lines.len());
                        let mut out = String::new();
                        out.push_str(&format!("--- a/{}\n", path));
                        out.push_str(&format!("+++ b/{}\n", path));
                        let mut shown = 0usize;
                        for i in 0..max_len {
                            let o = old_lines.get(i).copied().unwrap_or("");
                            let n = new_lines.get(i).copied().unwrap_or("");
                            if o != n {
                                out.push_str(&format!("@@ -{},{} +{},{} @@\n", i + 1, 1, i + 1, 1));
                                out.push_str(&format!("-{}\n", o));
                                out.push_str(&format!("+{}\n", n));
                                shown += 1;
                                if shown >= 20 {
                                    out.push_str(&i18n::t(
                                        "ui.confirm.msg.diff_truncated",
                                        "...（差异已截断）\n",
                                        "... (diff truncated)\n",
                                    ));
                                    break;
                                }
                            }
                        }
                        if shown == 0 {
                            out.push_str(&i18n::t(
                                "ui.confirm.msg.no_diff",
                                "（无差异或替换未匹配）\n",
                                "(no differences or replacement not matched)\n",
                            ));
                        }
                        out
                    }
                    Err(e) => {
                        let tpl = i18n::t(
                            "ui.confirm.msg.failed_read",
                            "（无法读取文件，无法生成差异: {0}）\n- {1}\n+ {2}",
                            "(failed to read file, cannot generate diff: {0})\n- {1}\n+ {2}",
                        );
                        tpl.replace("{0}", &e.to_string())
                            .replace("{1}", &old_str)
                            .replace("{2}", &new_str)
                    }
                }
            } else {
                let old_preview = old_str.lines().take(6).collect::<Vec<_>>().join("\n");
                let new_preview = new_str.lines().take(6).collect::<Vec<_>>().join("\n");
                {
                    let tpl = i18n::t(
                        "ui.confirm.msg.preview_mode",
                        "（预览模式: 参数不完整）\n- {0}\n+ {1}",
                        "(preview mode: incomplete parameters)\n- {0}\n+ {1}",
                    );
                    tpl.replace("{0}", &old_preview)
                        .replace("{1}", &new_preview)
                }
            };

            crate::types::ToolConfirmation {
                tool_name: "smart_edit".to_string(),
                operation_type: crate::types::ConfirmationType::EditFile,
                details: crate::types::ConfirmationDetails::EditFile {
                    file_path: path,
                    diff,
                    old_lines: old_str.lines().count(),
                    new_lines: new_str.lines().count(),
                },
                is_dangerous: false,
                outcome: None,
            }
        }
        "save_memory" => {
            let fact = get_str(&["fact"]).unwrap_or_else(|| "unknown".to_string());

            // Calculate path
            let mut path_buf =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            path_buf.push(".star");
            path_buf.push("memory.md");
            let path = path_buf.to_string_lossy().to_string();

            // Read and generate diff
            let (diff, old_lines_count, new_lines_count) =
                match tokio::fs::read_to_string(&path_buf).await {
                    Ok(content) => {
                        let mut new_content = content.clone();
                        if !new_content.is_empty() && !new_content.ends_with('\n') {
                            new_content.push('\n');
                        }
                        // Note: the timestamp here may be slightly different from actual execution time, but acceptable for preview
                        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                        let new_line = format!("- [{}] {}\n", now, fact);
                        new_content.push_str(&new_line);

                        let old_lines: Vec<&str> = content.lines().collect();
                        let new_lines: Vec<&str> = new_content.lines().collect();

                        let mut out = String::new();
                        out.push_str(&format!("--- a/{}\n", path));
                        out.push_str(&format!("+++ b/{}\n", path));

                        // Append diff always shows at the end
                        let start_line = old_lines.len();
                        out.push_str(&format!("@@ -{},0 +{},1 @@\n", start_line, start_line + 1));
                        out.push_str(&format!("+{}\n", new_line.trim_end()));

                        (out, old_lines.len(), new_lines.len())
                    }
                    Err(_) => {
                        // File may not exist, treat as new file or first write
                        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                        let new_line = format!("- [{}] {}\n", now, fact);
                        let out = format!(
                            "--- /dev/null\n+++ b/{}\n@@ -0,0 +1 @@\n+{}",
                            path,
                            new_line.trim_end()
                        );
                        (out, 0, 1)
                    }
                };

            crate::types::ToolConfirmation {
                tool_name: "save_memory".to_string(),
                operation_type: crate::types::ConfirmationType::EditFile,
                details: crate::types::ConfirmationDetails::EditFile {
                    file_path: path,
                    diff,
                    old_lines: old_lines_count,
                    new_lines: new_lines_count,
                },
                is_dangerous: false,
                outcome: None,
            }
        }
        "Bash" | "shell" => {
            let command =
                get_str(&["command", "CommandLine"]).unwrap_or_else(|| "unknown".to_string());
            let working_dir = get_str(&["dir_path", "working_dir"]).unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .unwrap_or_else(|| ".".to_string())
            });
            let risk = crate::agent::policies::tool_gate::estimate_bash_risk(&command);
            let is_dangerous = matches!(
                risk,
                crate::types::RiskLevel::Medium
                    | crate::types::RiskLevel::High
                    | crate::types::RiskLevel::Critical
            );
            // Auto-fetch git diff for commit commands
            let diff_preview = if command.contains("git") && command.contains("commit") {
                let stat_output = tokio::process::Command::new("git")
                    .args(["diff", "--cached", "--stat"])
                    .output()
                    .await;
                let diff_output = tokio::process::Command::new("git")
                    .args(["diff", "--cached"])
                    .output()
                    .await;
                match (stat_output, diff_output) {
                    (Ok(stat), Ok(diff)) => {
                        let stat_text = String::from_utf8_lossy(&stat.stdout).to_string();
                        let diff_text = String::from_utf8_lossy(&diff.stdout).to_string();
                        if stat_text.is_empty() && diff_text.is_empty() {
                            Some(i18n::t(
                                "ui.confirm.git_diff.no_changes",
                                "（无暂存变更）",
                                "(No staged changes)",
                            ))
                        } else {
                            Some(format!("{}\n{}", stat_text.trim(), diff_text.trim()))
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };
            crate::types::ToolConfirmation {
                tool_name: tc.function.name.clone(),
                operation_type: crate::types::ConfirmationType::ShellCommand,
                details: crate::types::ConfirmationDetails::ShellCommand {
                    command: command.to_string(),
                    working_dir,
                    estimated_risk: risk,
                    diff_preview,
                },
                is_dangerous,
                outcome: None,
            }
        }
        "create_file" => {
            let path = get_str(&["path", "file_path", "target_file"])
                .unwrap_or_else(|| "unknown".to_string());
            let content = get_str(&["content"]).unwrap_or_default();
            let preview_lines: Vec<&str> = content.lines().take(20).collect();
            let preview = if content.lines().count() > 20 {
                {
                    let tpl = i18n::t(
                        "ui.confirm.msg.remaining_omitted",
                        "{0}\n...（其余 {1} 行已省略）",
                        "{0}\n... (remaining {1} lines omitted)",
                    );
                    tpl.replace("{0}", &preview_lines.join("\n"))
                        .replace("{1}", &(content.lines().count() - 20).to_string())
                }
            } else {
                preview_lines.join("\n")
            };
            crate::types::ToolConfirmation {
                tool_name: "create_file".to_string(),
                operation_type: crate::types::ConfirmationType::CreateFile,
                details: crate::types::ConfirmationDetails::CreateFile {
                    file_path: path,
                    content_preview: preview,
                },
                is_dangerous: false,
                outcome: None,
            }
        }
        "view_file" | "Read" => {
            let path = get_str(&["path", "file_path", "target_file"])
                .unwrap_or_else(|| "unknown".to_string());

            let range = if tc.function.name == "view_file" {
                let start_line = get_u64(&["start_line"]).map(|v| v as usize);
                let end_line = get_u64(&["end_line"]).map(|v| v as usize);
                match (start_line, end_line) {
                    (Some(s), Some(e)) => format!(" [{}-{}]", s, e),
                    (Some(s), None) => format!(" [{}-]", s),
                    (None, Some(e)) => format!(" [-{}]", e),
                    (None, None) => String::new(),
                }
            } else {
                let offset = get_u64(&["offset"]).map(|v| v as usize);
                let limit = get_u64(&["limit"]).map(|v| v as usize);
                match (offset, limit) {
                    (Some(o), Some(l)) => format!(" [offset: {}, limit: {}]", o, l),
                    (Some(o), None) => format!(" [offset: {}]", o),
                    (None, Some(l)) => format!(" [limit: {}]", l),
                    (None, None) => String::new(),
                }
            };

            crate::types::ToolConfirmation {
                tool_name: tc.function.name.clone(),
                operation_type: crate::types::ConfirmationType::ShellCommand,
                details: crate::types::ConfirmationDetails::ShellCommand {
                    command: format!("{} {}{}", tc.function.name, path, range),
                    working_dir: std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| ".".to_string()),
                    estimated_risk: crate::types::RiskLevel::Low,
                    diff_preview: None,
                },
                is_dangerous: false,
                outcome: None,
            }
        }
        _ => crate::types::ToolConfirmation {
            tool_name: tc.function.name.clone(),
            operation_type: crate::types::ConfirmationType::ShellCommand,
            details: crate::types::ConfirmationDetails::ShellCommand {
                command: {
                    let args_preview = tc.function.arguments.chars().take(240).collect::<String>();
                    {
                        let tpl = i18n::t(
                            "ui.confirm.msg.unknown_tool",
                            "未知工具: {0} {1}",
                            "unknown tool: {0} {1}",
                        );
                        tpl.replace("{0}", &tc.function.name)
                            .replace("{1}", &args_preview)
                    }
                },
                working_dir: ".".to_string(),
                estimated_risk: crate::types::RiskLevel::Low,
                diff_preview: None,
            },
            is_dangerous: false,
            outcome: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ConfirmationDetails, ConfirmationType, RiskLevel, ToolConfirmation};

    fn conf(operation_type: ConfirmationType, details: ConfirmationDetails) -> ToolConfirmation {
        ToolConfirmation {
            tool_name: "T".to_string(),
            operation_type,
            details,
            is_dangerous: false,
            outcome: None,
        }
    }

    /// Bash 卡片的风险来自 `estimate_bash_risk`，这里锚定两端：
    /// `rm -rf /` 必须是 Critical，纯读命令不能被报成高风险。
    #[test]
    fn bash_risk_spans_critical_to_safe() {
        use crate::agent::policies::tool_gate::estimate_bash_risk;
        assert_eq!(estimate_bash_risk("rm -rf /"), RiskLevel::Critical);
        assert_eq!(estimate_bash_risk("rm -fr /tmp/x"), RiskLevel::Critical);
        assert_eq!(estimate_bash_risk("pwd"), RiskLevel::Low);
        assert_eq!(estimate_bash_risk("whoami"), RiskLevel::Low);
    }

    /// 非 Shell 卡片的分级：写到系统路径比写到项目里高一级。
    #[test]
    fn details_risk_escalates_on_system_paths() {
        let del_project = conf(
            ConfirmationType::DeleteFile,
            ConfirmationDetails::DeleteFile {
                file_path: "src/main.rs".to_string(),
            },
        );
        assert_eq!(estimate_details_risk(&del_project), RiskLevel::High);

        let del_system = conf(
            ConfirmationType::DeleteFile,
            ConfirmationDetails::DeleteFile {
                file_path: "/etc/passwd".to_string(),
            },
        );
        assert_eq!(estimate_details_risk(&del_system), RiskLevel::Critical);

        let edit_project = conf(
            ConfirmationType::EditFile,
            ConfirmationDetails::EditFile {
                file_path: "src/main.rs".to_string(),
                diff: String::new(),
                old_lines: 1,
                new_lines: 2,
            },
        );
        assert_eq!(estimate_details_risk(&edit_project), RiskLevel::Low);

        let edit_system = conf(
            ConfirmationType::EditFile,
            ConfirmationDetails::EditFile {
                file_path: "/etc/hosts".to_string(),
                diff: String::new(),
                old_lines: 1,
                new_lines: 2,
            },
        );
        assert_eq!(estimate_details_risk(&edit_system), RiskLevel::High);

        let net = conf(
            ConfirmationType::NetworkRequest,
            ConfirmationDetails::NetworkRequest {
                url: "https://example.com".to_string(),
                method: "GET".to_string(),
            },
        );
        assert_eq!(estimate_details_risk(&net), RiskLevel::Medium);
    }

    /// 卡片上只能出现一条 Risk 行：Shell 由 details 分支打印，其余类型由新增分支打印。
    #[test]
    fn card_renders_exactly_one_risk_line() {
        let count_risk = |c: &ToolConfirmation| {
            build_confirmation_card_block(c, 60, 0, false, false)
                .iter()
                .filter(|l| {
                    let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                    text.contains("Risk:") || text.contains("风险:")
                })
                .count()
        };

        let shell = conf(
            ConfirmationType::ShellCommand,
            ConfirmationDetails::ShellCommand {
                command: "rm -rf /tmp/x".to_string(),
                working_dir: ".".to_string(),
                estimated_risk: RiskLevel::Critical,
                diff_preview: None,
            },
        );
        assert_eq!(count_risk(&shell), 1);

        let edit = conf(
            ConfirmationType::EditFile,
            ConfirmationDetails::EditFile {
                file_path: "/etc/hosts".to_string(),
                diff: String::new(),
                old_lines: 1,
                new_lines: 2,
            },
        );
        assert_eq!(count_risk(&edit), 1);
    }
}
