/// Log selector — interactive session browser for resuming past conversations.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::core::i18n;
use crate::ui::themes::theme::Theme;

/// Session entry for display in the log selector
#[derive(Debug, Clone)]
pub struct LogSessionEntry {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub message_count: usize,
    pub preview: String,
}

/// Log selector state
#[derive(Debug, Clone, Default)]
pub struct LogSelectorState {
    pub sessions: Vec<LogSessionEntry>,
    pub selected_index: usize,
    pub preview_scroll: usize,
    pub is_loading: bool,
    pub search_query: String,
}

impl LogSelectorState {
    pub fn filtered_sessions(&self) -> Vec<(usize, &LogSessionEntry)> {
        if self.search_query.is_empty() {
            self.sessions.iter().enumerate().collect()
        } else {
            let query = self.search_query.to_lowercase();
            self.sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.title.to_lowercase().contains(&query)
                        || s.preview.to_lowercase().contains(&query)
                })
                .collect()
        }
    }

    pub fn select_next(&mut self) {
        let count = self.filtered_sessions().len();
        if count > 0 {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    pub fn select_prev(&mut self) {
        let count = self.filtered_sessions().len();
        if count > 0 {
            self.selected_index = if self.selected_index == 0 {
                count - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub fn get_selected_session(&self) -> Option<&LogSessionEntry> {
        self.filtered_sessions()
            .get(self.selected_index)
            .map(|(_, s)| *s)
    }
}

/// Render the log selector overlay
pub fn render_log_selector(f: &mut Frame, state: &LogSelectorState, area: Rect, theme: &Theme) {
    let width = 80.min(area.width.saturating_sub(4));
    let height = 24.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect { x, y, width, height };

    f.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Title + search
            Constraint::Min(10),  // List + preview
            Constraint::Length(1), // Hints
        ])
        .split(popup_area);

    // Title + search
    let title_line = Line::from(vec![
        Span::styled(
            format!(" {} ", i18n::t("ui.log_selector.title", "会话浏览器", "Session Browser")),
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  /{}", state.search_query),
            Style::default().fg(theme.secondary),
        ),
    ]);
    f.render_widget(Paragraph::new(title_line), chunks[0]);

    // List + preview
    let inner_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(60),
        ])
        .split(chunks[1]);

    // Session list
    let filtered = state.filtered_sessions();
    let items: Vec<ListItem> = filtered
        .iter()
        .map(|(_, session)| {
            let style = Style::default().fg(theme.foreground);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{} ", session.title),
                    style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("({})", session.created_at),
                    Style::default().fg(theme.comment),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(
                    format!(" {} ", i18n::t("ui.log_selector.sessions", "会话", "Sessions")),
                    Style::default().fg(theme.secondary),
                )),
        )
        .highlight_style(Style::default().bg(theme.selection_bg).fg(theme.foreground))
        .highlight_symbol("❯ ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_index));
    f.render_stateful_widget(list, inner_chunks[0], &mut list_state);

    // Preview
    let preview_text = if let Some(session) = state.get_selected_session() {
        if session.preview.is_empty() {
            i18n::t("ui.log_selector.no_preview", "无预览", "No preview")
        } else {
            session.preview.clone()
        }
    } else {
        i18n::t("ui.log_selector.no_sessions", "无保存的会话", "No saved sessions")
    };

    let preview = Paragraph::new(preview_text)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(
                    format!(" {} ", i18n::t("ui.log_selector.preview", "预览", "Preview")),
                    Style::default().fg(theme.secondary),
                )),
        );
    f.render_widget(preview, inner_chunks[1]);

    // Hints
    let hints = Line::from(vec![
        Span::styled(
            format!(
                " Enter={} Esc={} /={}",
                i18n::t("ui.log_selector.resume", "恢复", "Resume"),
                i18n::t("ui.log_selector.close", "关闭", "Close"),
                i18n::t("ui.log_selector.filter", "筛选", "Filter"),
            ),
            Style::default().fg(theme.comment),
        ),
    ]);
    f.render_widget(Paragraph::new(hints), chunks[2]);
}
