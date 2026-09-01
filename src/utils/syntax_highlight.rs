use once_cell::sync::Lazy;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Global syntax definitions (loaded once, shared across all highlighting calls)
static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);

/// Global theme definitions (loaded once, shared across all highlighting calls)
static THEME_SET: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);

/// Convert syntect color to ratatui color
fn to_ratatui_color(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

/// Convert syntect style to ratatui style
fn to_ratatui_style(s: SynStyle) -> Style {
    let mut style = Style::default().fg(to_ratatui_color(s.foreground));
    let font = s.font_style;
    if font.contains(syntect::highlighting::FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if font.contains(syntect::highlighting::FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if font.contains(syntect::highlighting::FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

/// Map common language identifiers to syntect syntax names.
/// Returns None for unknown languages (will use plain text fallback).
fn find_syntax<'a>(language: &str) -> &'a syntect::parsing::SyntaxReference {
    let name = match language {
        "rust" | "rs" => "Rust",
        "python" | "py" => "Python",
        "javascript" | "js" => "JavaScript",
        "typescript" | "ts" => "TypeScript",
        "go" => "Go",
        "c" => "C",
        "cpp" | "c++" | "cxx" => "C++",
        "java" => "Java",
        "html" => "HTML",
        "css" => "CSS",
        "scss" | "sass" => "SCSS",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "xml" => "XML",
        "markdown" | "md" => "Markdown",
        "bash" | "sh" | "shell" | "zsh" => "Shell-Unix-Generic",
        "sql" => "SQL",
        "ruby" | "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "kotlin" | "kt" => "Kotlin",
        "scala" => "Scala",
        "lua" => "Lua",
        "r" => "R",
        "dart" => "Dart",
        "elixir" | "ex" => "Elixir",
        "haskell" | "hs" => "Haskell",
        "dockerfile" | "docker" => "Dockerfile",
        "makefile" | "make" => "Makefile",
        "cmake" => "CMake",
        "ini" => "INI",
        _ => "Plain Text",
    };
    SYNTAX_SET
        .find_syntax_by_name(name)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text())
}

/// Highlight a single diff line with standard diff coloring:
/// - Lines starting with `+` (not `+++`): green
/// - Lines starting with `-` (not `---`): red
/// - Lines starting with `@@`: cyan
/// - File headers (`+++`, `---`): bold cyan
/// - Context: dark gray
fn highlight_diff_line(line: &str) -> Line<'static> {
    let style = if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if line.starts_with("+++") || line.starts_with("---") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Line::from(Span::styled(line.to_string(), style))
}

/// Highlight a single line of code using syntect's TextMate grammar engine.
///
/// For diff/patch languages, uses custom diff coloring instead.
/// For unknown languages, falls back to plain text.
///
/// Note: This creates a new `HighlightLines` per call. For batch highlighting
/// of multi-line code blocks, use `highlight_code_block` instead.
pub fn highlight_line(line: &str, language: &str) -> Line<'static> {
    if language == "diff" || language == "patch" {
        return highlight_diff_line(line);
    }

    let syntax = find_syntax(language);
    let theme = &THEME_SET.themes["base16-ocean.dark"];
    let mut highlighter = HighlightLines::new(syntax, theme);

    let ranges = highlighter
        .highlight_line(line, &SYNTAX_SET)
        .unwrap_or_default();
    let spans: Vec<Span<'static>> = ranges
        .into_iter()
        .map(|(style, text)| Span::styled(text.to_string(), to_ratatui_style(style)))
        .collect();

    if spans.is_empty() {
        Line::from(Span::raw(line.to_string()))
    } else {
        Line::from(spans)
    }
}

/// Highlight a full code block (multiple lines) using a single `HighlightLines` instance.
///
/// This is more efficient than calling `highlight_line` per line because:
/// 1. Single `HighlightLines` instance (avoids repeated initialization)
/// 2. Maintains state across lines (e.g. multi-line comments)
///
/// Returns a Vec of ratatui `Line`s with syntax highlighting.
pub fn highlight_block(code: &str, language: &str) -> Vec<Line<'static>> {
    if language == "diff" || language == "patch" {
        return code.lines().map(highlight_diff_line).collect();
    }

    let syntax = find_syntax(language);
    let theme = &THEME_SET.themes["base16-ocean.dark"];
    let mut highlighter = HighlightLines::new(syntax, theme);

    code.lines()
        .map(|line| {
            let ranges = highlighter
                .highlight_line(line, &SYNTAX_SET)
                .unwrap_or_default();
            let spans: Vec<Span<'static>> = ranges
                .into_iter()
                .map(|(style, text)| Span::styled(text.to_string(), to_ratatui_style(style)))
                .collect();

            if spans.is_empty() {
                Line::from(Span::raw(line.to_string()))
            } else {
                Line::from(spans)
            }
        })
        .collect()
}
