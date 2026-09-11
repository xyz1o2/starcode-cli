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

impl From<crate::utils::session_manager::SessionSummary> for LogSessionEntry {
    fn from(summary: crate::utils::session_manager::SessionSummary) -> Self {
        Self {
            id: summary.id,
            title: summary.title,
            created_at: summary.subtitle.clone(),
            message_count: 0,
            preview: summary.subtitle,
        }
    }
}

/// Log selector state
#[derive(Debug, Clone, Default)]
pub struct LogSelectorState {
    pub sessions: Vec<LogSessionEntry>,
    pub selected_index: usize,
    pub preview_scroll: usize,
    pub is_loading: bool,
    pub load_error: Option<String>,
    pub search_query: String,
}

impl LogSelectorState {
    /// 打开选择器前清理旧查询和结果，避免上一项目状态泄漏到本次加载。
    pub fn begin_loading(&mut self) {
        self.sessions.clear();
        self.selected_index = 0;
        self.preview_scroll = 0;
        self.search_query.clear();
        self.load_error = None;
        self.is_loading = true;
    }

    /// 用新的持久化会话列表替换旧结果并回到第一项。
    pub fn set_sessions(&mut self, sessions: Vec<LogSessionEntry>) {
        self.sessions = sessions;
        self.selected_index = 0;
        self.preview_scroll = 0;
        self.is_loading = false;
    }

    /// 记录加载错误，让空会话与读取失败在 UI 中可区分。
    pub fn set_load_error(&mut self, error: String) {
        self.sessions.clear();
        self.selected_index = 0;
        self.preview_scroll = 0;
        self.is_loading = false;
        self.load_error = Some(error);
    }
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
    let popup_area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Title + search
            Constraint::Min(10),   // List + preview
            Constraint::Length(1), // Hints
        ])
        .split(popup_area);

    // Title + search
    let title_line = Line::from(vec![
        Span::styled(
            format!(
                " {} ",
                i18n::t("ui.log_selector.title", "会话浏览器", "Session Browser")
            ),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
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
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
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
                    format!(
                        " {} ",
                        i18n::t("ui.log_selector.sessions", "会话", "Sessions")
                    ),
                    Style::default().fg(theme.secondary),
                )),
        )
        .highlight_style(Style::default().bg(theme.selection_bg).fg(theme.foreground))
        .highlight_symbol("❯ ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_index));
    f.render_stateful_widget(list, inner_chunks[0], &mut list_state);

    // Preview
    let preview_text = if state.is_loading {
        i18n::t(
            "ui.log_selector.loading",
            "正在加载保存的会话…",
            "Loading saved sessions...",
        )
    } else if let Some(error) = state.load_error.as_deref() {
        format!(
            "{}: {}",
            i18n::t(
                "ui.log_selector.load_failed",
                "无法加载会话",
                "Could not load sessions"
            ),
            error
        )
    } else if let Some(session) = state.get_selected_session() {
        if session.preview.is_empty() {
            i18n::t("ui.log_selector.no_preview", "无预览", "No preview")
        } else {
            session.preview.clone()
        }
    } else {
        i18n::t(
            "ui.log_selector.no_sessions",
            "无保存的会话",
            "No saved sessions",
        )
    };

    let preview = Paragraph::new(preview_text)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme.border))
                .title(Span::styled(
                    format!(
                        " {} ",
                        i18n::t("ui.log_selector.preview", "预览", "Preview")
                    ),
                    Style::default().fg(theme.secondary),
                )),
        );
    f.render_widget(preview, inner_chunks[1]);

    // Hints
    let hints = Line::from(vec![Span::styled(
        format!(
            " Enter={} Esc={} /={}",
            i18n::t("ui.log_selector.resume", "恢复", "Resume"),
            i18n::t("ui.log_selector.close", "关闭", "Close"),
            i18n::t("ui.log_selector.filter", "筛选", "Filter"),
        ),
        Style::default().fg(theme.comment),
    )]);
    f.render_widget(Paragraph::new(hints), chunks[2]);
}

#[cfg(test)]
mod tests {
    use super::{LogSelectorState, LogSessionEntry};
    use crate::utils::session_manager::SessionSummary;

    fn entry(id: &str, title: &str) -> LogSessionEntry {
        LogSessionEntry {
            id: id.to_string(),
            title: title.to_string(),
            created_at: "09-08 12:00".to_string(),
            message_count: 1,
            preview: format!("{} preview", title),
        }
    }

    #[test]
    fn summary_conversion_preserves_browser_identity_and_metadata() {
        let entry = LogSessionEntry::from(SessionSummary {
            id: "auto-123".to_string(),
            title: "Investigate rendering".to_string(),
            subtitle: "Latest · 09-08 12:00 · 3 msgs".to_string(),
            created_at: 0,
        });

        assert_eq!(entry.id, "auto-123");
        assert_eq!(entry.title, "Investigate rendering");
        assert_eq!(entry.created_at, "Latest · 09-08 12:00 · 3 msgs");
        assert_eq!(entry.preview, "Latest · 09-08 12:00 · 3 msgs");
    }

    #[test]
    fn loading_and_refresh_reset_selector_state() {
        let mut state = LogSelectorState {
            sessions: vec![entry("old", "Old")],
            selected_index: 3,
            preview_scroll: 4,
            is_loading: false,
            load_error: Some("old error".to_string()),
            search_query: "old".to_string(),
        };

        state.begin_loading();
        assert!(state.is_loading);
        assert!(state.sessions.is_empty());
        assert_eq!(state.selected_index, 0);
        assert_eq!(state.preview_scroll, 0);
        assert!(state.search_query.is_empty());
        assert!(state.load_error.is_none());

        state.set_sessions(vec![entry("new", "New")]);
        assert!(!state.is_loading);
        assert_eq!(
            state
                .get_selected_session()
                .map(|session| session.id.as_str()),
            Some("new")
        );
    }

    #[test]
    fn selection_tracks_filtered_rows() {
        let mut state = LogSelectorState::default();
        state.set_sessions(vec![entry("one", "Alpha"), entry("two", "Beta")]);
        state.search_query = "beta".to_string();

        assert_eq!(
            state
                .get_selected_session()
                .map(|session| session.id.as_str()),
            Some("two")
        );
        state.select_next();
        assert_eq!(
            state
                .get_selected_session()
                .map(|session| session.id.as_str()),
            Some("two")
        );
    }
}
