/// Structured diff rendering with syntax highlighting.
///
/// Provides terminal-friendly diff visualization with:
/// - Line numbers (gutter)
/// - Syntax highlighting
/// - Add/remove/modify indicators
/// - Context lines
use super::{detect_language, get_highlighter};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Diff line type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineType {
    Added,
    Removed,
    Context,
    Header,
}

/// A single line in a diff
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub line_type: DiffLineType,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
}

/// Parse a unified diff into structured lines
pub fn parse_diff(diff_text: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    let mut old_line = 0;
    let mut new_line = 0;

    for line in diff_text.lines() {
        if line.starts_with("@@") {
            // Parse hunk header: @@ -old_start,old_count +new_start,new_count @@
            let diff_line = DiffLine {
                line_type: DiffLineType::Header,
                old_line: None,
                new_line: None,
                content: line.to_string(),
            };
            lines.push(diff_line);

            // Extract line numbers from hunk header
            if let Some(caps) = parse_hunk_header(line) {
                old_line = caps.0;
                new_line = caps.1;
            }
        } else if line.starts_with('+') {
            lines.push(DiffLine {
                line_type: DiffLineType::Added,
                old_line: None,
                new_line: Some(new_line),
                content: line[1..].to_string(),
            });
            new_line += 1;
        } else if line.starts_with('-') {
            lines.push(DiffLine {
                line_type: DiffLineType::Removed,
                old_line: Some(old_line),
                new_line: None,
                content: line[1..].to_string(),
            });
            old_line += 1;
        } else if line.starts_with(' ') || line.is_empty() {
            lines.push(DiffLine {
                line_type: DiffLineType::Context,
                old_line: Some(old_line),
                new_line: Some(new_line),
                content: line.strip_prefix(' ').unwrap_or(line).to_string(),
            });
            old_line += 1;
            new_line += 1;
        }
    }

    lines
}

/// Parse hunk header to extract starting line numbers
fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    // Format: @@ -old_start,old_count +new_start,new_count @@
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let old = parts[1].strip_prefix('-')?;
    let new = parts[2].strip_prefix('+')?;
    let old_start = old.split(',').next()?.parse().ok()?;
    let new_start = new.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

/// Render a diff as ratatui Lines with syntax highlighting
pub fn render_diff_lines(diff_text: &str, filename: &str, width: u16) -> Vec<Line<'static>> {
    let diff_lines = parse_diff(diff_text);
    let language = detect_language(filename);
    let highlighter = get_highlighter();
    let mut result = Vec::new();

    // Gutter width: "  123 | " = 8 chars
    let gutter_width = 8;
    let content_width = (width as usize).saturating_sub(gutter_width + 1);

    for diff_line in &diff_lines {
        match diff_line.line_type {
            DiffLineType::Header => {
                result.push(Line::from(vec![
                    Span::styled(
                        format!("  {:width$} ", "", width = gutter_width - 2),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        diff_line.content.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            DiffLineType::Added => {
                let highlighted = highlighter.highlight_line(&diff_line.content, language);
                let line_num = diff_line
                    .new_line
                    .map(|n| format!("{:>4}", n))
                    .unwrap_or_else(|| "    ".to_string());
                result.push(Line::from(vec![
                    Span::styled(
                        format!("{} │ ", line_num),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        "+ ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        truncate_ansi(&highlighted, content_width),
                        Style::default().fg(Color::Green),
                    ),
                ]));
            }
            DiffLineType::Removed => {
                let highlighted = highlighter.highlight_line(&diff_line.content, language);
                let line_num = diff_line
                    .old_line
                    .map(|n| format!("{:>4}", n))
                    .unwrap_or_else(|| "    ".to_string());
                result.push(Line::from(vec![
                    Span::styled(
                        format!("{} │ ", line_num),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        "- ",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        truncate_ansi(&highlighted, content_width),
                        Style::default().fg(Color::Red),
                    ),
                ]));
            }
            DiffLineType::Context => {
                let highlighted = highlighter.highlight_line(&diff_line.content, language);
                let old_num = diff_line
                    .old_line
                    .map(|n| format!("{:>4}", n))
                    .unwrap_or_else(|| "    ".to_string());
                let new_num = diff_line
                    .new_line
                    .map(|n| format!("{:>4}", n))
                    .unwrap_or_else(|| "    ".to_string());
                result.push(Line::from(vec![
                    Span::styled(
                        format!("{} │ ", old_num),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled("  ", Style::default()),
                    Span::styled(
                        truncate_ansi(&highlighted, content_width),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }

    result
}

/// Truncate a string with ANSI codes to a maximum visible width
fn truncate_ansi(s: &str, max_width: usize) -> String {
    let mut visible_count = 0;
    let mut in_escape = false;
    let mut result = String::new();

    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
            result.push(ch);
        } else if in_escape {
            result.push(ch);
            if ch == 'm' {
                in_escape = false;
            }
        } else {
            if visible_count >= max_width {
                break;
            }
            result.push(ch);
            visible_count += 1;
        }
    }

    // Reset color at end
    if !result.ends_with("\x1b[0m") {
        result.push_str("\x1b[0m");
    }

    result
}

/// Render a simple diff without syntax highlighting (fallback)
pub fn render_diff_simple(diff_text: &str) -> Vec<Line<'static>> {
    let diff_lines = parse_diff(diff_text);
    let mut result = Vec::new();

    for diff_line in &diff_lines {
        let (prefix, color) = match diff_line.line_type {
            DiffLineType::Header => ("  ", Color::Cyan),
            DiffLineType::Added => ("+ ", Color::Green),
            DiffLineType::Removed => ("- ", Color::Red),
            DiffLineType::Context => ("  ", Color::DarkGray),
        };

        result.push(Line::from(vec![
            Span::styled(prefix, Style::default().fg(color)),
            Span::styled(diff_line.content.clone(), Style::default().fg(color)),
        ]));
    }

    result
}
