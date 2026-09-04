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
    let effort = effort_label(&state.thinking_effort);
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

/// thinking effort 的后缀；Off 不显示，避免抬头出现无意义的 "· off"。
fn effort_label(effort: &crate::types::ThinkingEffort) -> Option<&'static str> {
    use crate::types::ThinkingEffort;
    match effort {
        ThinkingEffort::Off => None,
        ThinkingEffort::Low => Some("thinking low"),
        ThinkingEffort::Medium => Some("thinking medium"),
        ThinkingEffort::High => Some("thinking high"),
    }
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
    fn effort_off_adds_no_suffix() {
        assert_eq!(effort_label(&crate::types::ThinkingEffort::Off), None);
    }
}
