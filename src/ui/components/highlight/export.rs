/// Export functionality — export conversation to file or clipboard.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Export format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Markdown,
    Json,
    Text,
}

impl ExportFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Json => "json",
            ExportFormat::Text => "txt",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ExportFormat::Markdown => "Markdown",
            ExportFormat::Json => "JSON",
            ExportFormat::Text => "Plain Text",
        }
    }
}

/// Export state
#[derive(Debug)]
pub struct ExportState {
    pub format: ExportFormat,
    pub filename: String,
    pub include_metadata: bool,
    pub include_tool_calls: bool,
}

impl ExportState {
    pub fn new() -> Self {
        Self {
            format: ExportFormat::Markdown,
            filename: format!("export-{}.md", chrono::Local::now().format("%Y%m%d-%H%M%S")),
            include_metadata: true,
            include_tool_calls: true,
        }
    }

    pub fn cycle_format(&mut self) {
        self.format = match self.format {
            ExportFormat::Markdown => ExportFormat::Json,
            ExportFormat::Json => ExportFormat::Text,
            ExportFormat::Text => ExportFormat::Markdown,
        };
        self.filename = format!(
            "export-{}.{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            self.format.extension()
        );
    }
}

/// Render export dialog
pub fn render_export_dialog(f: &mut Frame, state: &ExportState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title
            Constraint::Length(1), // Format
            Constraint::Length(1), // Filename
            Constraint::Length(1), // Options
            Constraint::Length(1), // Help
        ])
        .split(area);

    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Export Conversation ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Title
    let title = Line::from(vec![
        Span::styled("  Export conversation to file", Style::default().fg(Color::White)),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // Format
    let format_line = Line::from(vec![
        Span::styled("  Format: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.format.name(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" (Tab to change)", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(format_line), chunks[1]);

    // Filename
    let filename_line = Line::from(vec![
        Span::styled("  File: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            state.filename.clone(),
            Style::default().fg(Color::Yellow),
        ),
    ]);
    f.render_widget(Paragraph::new(filename_line), chunks[2]);

    // Options
    let options_line = Line::from(vec![
        Span::styled("  Include metadata: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if state.include_metadata { "Yes" } else { "No" },
            Style::default().fg(if state.include_metadata { Color::Green } else { Color::Red }),
        ),
        Span::styled("  Include tool calls: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if state.include_tool_calls { "Yes" } else { "No" },
            Style::default().fg(if state.include_tool_calls { Color::Green } else { Color::Red }),
        ),
    ]);
    f.render_widget(Paragraph::new(options_line), chunks[3]);

    // Help
    let help_line = Line::from(vec![
        Span::styled(
            "  Enter Export  Esc Cancel  Tab Format  m Metadata  t Tool calls",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(help_line), chunks[4]);
}

/// Export conversation to markdown
pub fn export_to_markdown(
    history: &[(String, String)],
    include_metadata: bool,
    include_tool_calls: bool,
) -> String {
    let mut output = String::new();

    if include_metadata {
        output.push_str(&format!(
            "# Conversation Export\n\nExported: {}\n\n---\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ));
    }

    for (role, content) in history {
        if !include_tool_calls && role == "tool" {
            continue;
        }
        output.push_str(&format!("## {}\n\n{}\n\n", role.to_uppercase(), content));
    }

    output
}

/// Export conversation to JSON
pub fn export_to_json(
    history: &[(String, String)],
    include_metadata: bool,
    include_tool_calls: bool,
) -> Result<String, String> {
    let entries: Vec<serde_json::Value> = history
        .iter()
        .filter(|(role, _)| include_tool_calls || role != "tool")
        .map(|(role, content)| {
            serde_json::json!({
                "role": role,
                "content": content,
            })
        })
        .collect();

    let export = if include_metadata {
        serde_json::json!({
            "exported_at": chrono::Local::now().to_rfc3339(),
            "messages": entries,
        })
    } else {
        serde_json::json!(entries)
    };

    serde_json::to_string_pretty(&export).map_err(|e| e.to_string())
}
