//! 主界面顶部的欢迎抬头（对标 Claude Code 的 `CondensedLogo`）。
//!
//! 布局：左侧一个 3 行标记，右侧三行信息 —— 名称+版本 / 当前模型 / 工作目录。
//! 标记刻意不沿用 Claude Code 的吉祥物，也不用旧启动画面的大号块字 LOGO：
//! 那个 LOGO 只在 splash 里出现，而 splash 已经删掉了。
//!
//! 抬头是渲染期从 `ChatState` 现算的，不是启动时拼好的字符串 ——
//! 这样 `/model` 切模型后抬头会跟着变，而且启动阶段（模型还没解析出来）
//! 也能先画出来，占位显示 `...`。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::state::ChatState;

/// 左侧标记：一个四角星火花。
///
/// 字符只能从东亚宽度为 Neutral 的块元素里挑 —— 四分之一块 U+2596–U+259F
/// 和浅色阴影 U+2591 是 Neutral，在 `width_cjk` 下仍占 1 列；而半块/八分块
/// U+2580–U+258F（▀ ▄ █ ▌ 之类）是 Ambiguous，CJK 规则下算 2 列，会把右侧
/// 文字推歪。所以星芯只能用 ░ 而不是 █：整格填满的实心块没有 Neutral 版本。
const MARK: [&str; 3] = ["▚ ▞", " ░ ", "▞ ▚"];

/// 标记左边的留白列数，和聊天正文的左边距对齐。
const LEFT_PAD: &str = " ";
/// 标记与右侧文字之间的间隔。
const GAP: &str = "   ";

/// 组装抬头的行。返回值直接进 `chat_history` 的渲染块。
pub fn welcome_header_lines(state: &ChatState) -> Vec<Line<'static>> {
    let theme = state.theme_manager.current();
    let mark_style = Style::default().fg(theme.primary);

    let model = if state.current_model.is_empty() {
        "...".to_string()
    } else {
        state.current_model.clone()
    };
    let effort = effort_label(&state.thinking_effort, &state.current_model);
    let model_line = match effort {
        Some(label) => format!("{} · {}", model, label),
        None => model,
    };

    let cwd = std::env::current_dir()
        .map(|p| compact_home(&p.to_string_lossy()))
        .unwrap_or_else(|_| ".".to_string());

    // 右侧三行，与左侧标记逐行配对
    let info: [Vec<Span<'static>>; 3] = [
        vec![
            Span::styled(
                "StarCode",
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(theme.secondary),
            ),
        ],
        vec![Span::styled(
            model_line,
            Style::default().fg(theme.secondary),
        )],
        vec![Span::styled(cwd, Style::default().fg(theme.secondary))],
    ];

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(5);
    for (mark, spans) in MARK.iter().zip(info) {
        let mut row = vec![
            Span::styled(LEFT_PAD.to_string(), Style::default()),
            Span::styled(mark.to_string(), mark_style),
            Span::styled(GAP.to_string(), Style::default()),
        ];
        row.extend(spans);
        lines.push(Line::from(row));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "{}/help for help  ·  Esc to interrupt  ·  --resume <id> to resume",
            LEFT_PAD
        ),
        Style::default().fg(theme.secondary),
    )));

    lines
}

/// 抬头是渲染期现算的，但渲染结果按条目缓存在 `state.rendered_cache` 里，且只在条目被
/// 标脏时才刷新。抬头里的信息（模型、思考档位）在别处被改动后，必须把承载抬头的那条
/// `is_welcome` 条目标脏，否则屏幕上留着的还是旧字符串。
pub fn invalidate(state: &mut ChatState) {
    if let Some(idx) = state.chat_history.iter().position(|e| e.is_welcome) {
        state.virtual_list.mark_dirty(idx);
    }
}

/// 抬头的内容指纹。抬头里会变的东西只有这两样：模型名（含"支不支持思考"这个派生判断）
/// 和思考档位；版本号是编译期常量，工作目录一个会话内不变。
fn fingerprint(state: &ChatState) -> String {
    format!(
        "{}\u{1}{}",
        state.current_model,
        state.thinking_effort.as_str()
    )
}

/// 渲染前调用：指纹变了就把抬头标脏。
///
/// 之所以在渲染路径上判而不是在赋值点上通知：`current_model` 全仓库有十几个赋值点
/// （`/model`、`/fast`、面板、流式响应回填的模型名……），漏一个就是一处静默的过期显示。
pub fn refresh_if_stale(state: &mut ChatState) {
    let fp = fingerprint(state);
    if state.welcome_header_fingerprint.as_deref() == Some(fp.as_str()) {
        return;
    }
    state.welcome_header_fingerprint = Some(fp);
    invalidate(state);
}

/// thinking effort 的后缀，例如 `◐ medium`。
///
/// 只要模型有思考开关就一直显示，包括默认的 Off。原来 Off 返回 `None`，而 Off 恰好是
/// 默认值 —— 于是没人动过档位时抬头里根本没有这一格，用户自然会问"调思考深度的 UI 在
/// 哪"。对标 Claude Code：档位常驻显示，只在模型不支持思考时才隐藏。
fn effort_label(effort: &crate::types::ThinkingEffort, model: &str) -> Option<String> {
    if !crate::core::config::models::supports_thinking_ui(model) {
        return None;
    }
    Some(effort.label())
}

/// 把家目录前缀折成 `~`，长路径不至于把抬头撑爆。
fn compact_home(path: &str) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn mark_rows_all_have_the_same_display_width() {
        // 三行标记必须等宽，否则右侧三行信息会呈锯齿状错开
        let widths: Vec<usize> = MARK.iter().map(|r| r.width()).collect();
        let first = widths[0];
        assert!(first > 0, "标记不能为空");
        assert!(
            widths.iter().all(|w| *w == first),
            "标记各行宽度不一致: {:?}",
            widths
        );
    }

    #[test]
    fn mark_is_width_stable_under_cjk_rules() {
        // 块元素字符在 width_cjk 下也必须是 1 列宽 —— 否则 CJK 终端里抬头会错位
        for row in MARK {
            assert_eq!(
                row.width(),
                row.width_cjk(),
                "标记在 CJK 宽度规则下变宽了: {:?}",
                row
            );
        }
    }

    #[test]
    fn compact_home_shortens_only_a_real_prefix() {
        assert_eq!(compact_home("/nowhere/else/x"), "/nowhere/else/x");
    }

    #[test]
    fn effort_is_shown_even_at_the_default_off() {
        // 默认档位就是 Off；这一格必须照样出现，否则用户翻遍界面也找不到思考深度在哪调
        assert_eq!(
            effort_label(&crate::types::ThinkingEffort::Off, "claude-opus-5"),
            Some("◌ off".to_string())
        );
    }

    #[test]
    fn effort_is_shown_before_the_model_name_resolves() {
        // 启动阶段模型名还是空串，先按"支持"处理，指示器早一点出现
        assert_eq!(
            effort_label(&crate::types::ThinkingEffort::Medium, ""),
            Some("◐ medium".to_string())
        );
    }

    #[test]
    fn a_model_without_thinking_hides_the_effort() {
        assert_eq!(
            effort_label(&crate::types::ThinkingEffort::High, "gpt-4o"),
            None
        );
    }
}
