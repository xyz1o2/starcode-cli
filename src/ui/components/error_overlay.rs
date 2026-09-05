/// Error overlay — displays provider errors with retry/cancel/switch options.
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::core::i18n;
use crate::ui::themes::theme::Theme;

/// Error type classification
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ErrorType {
    #[default]
    Unknown,
    RateLimit,
    AuthError,
    /// 欠费 / 配额耗尽。和 AuthError 分开：提示词不同（一个换 key，一个充钱），
    /// 而且它绝对不可重试 —— 以前它落在 ProviderError 里被当成可重试，
    /// 于是余额耗尽时界面会礼貌地重试 10 次再报同一个错。
    QuotaError,
    NetworkError,
    ProviderError,
    ContextOverflow,
}

/// Error action selection
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ErrorAction {
    #[default]
    Retry,
    Cancel,
    SwitchProvider,
}

/// Error overlay state
#[derive(Debug, Clone, Default)]
pub struct ErrorOverlayState {
    pub error_message: String,
    pub error_type: ErrorType,
    pub retry_count: u32,
    pub max_retries: u32,
    pub is_retrying: bool,
    pub selected_action: ErrorAction,
}

impl ErrorOverlayState {
    pub fn select_next(&mut self) {
        self.selected_action = match self.selected_action {
            ErrorAction::Retry => ErrorAction::Cancel,
            ErrorAction::Cancel => ErrorAction::SwitchProvider,
            ErrorAction::SwitchProvider => ErrorAction::Retry,
        };
    }

    pub fn select_prev(&mut self) {
        self.selected_action = match self.selected_action {
            ErrorAction::Retry => ErrorAction::SwitchProvider,
            ErrorAction::Cancel => ErrorAction::Retry,
            ErrorAction::SwitchProvider => ErrorAction::Cancel,
        };
    }
}

/// Classify an error string into an ErrorType.
///
/// 判定逻辑复用 `llm::error_kind` —— 和 client 层的诊断文案、恢复层的
/// 重试决策同源。以前这里自己按子串猜一遍，导致同一个 402 在 client 里
/// 有友好提示、在恢复层被压缩重试、在这里显示成"可重试的 Provider Error"。
pub fn classify_error(error: &str) -> ErrorType {
    use crate::llm::error_kind::LlmErrorKind;
    match LlmErrorKind::classify(error) {
        LlmErrorKind::Auth => ErrorType::AuthError,
        LlmErrorKind::Quota => ErrorType::QuotaError,
        LlmErrorKind::RateLimit => ErrorType::RateLimit,
        LlmErrorKind::Network => ErrorType::NetworkError,
        LlmErrorKind::Overloaded | LlmErrorKind::ServerError => ErrorType::ProviderError,
        LlmErrorKind::ContextWindow => ErrorType::ContextOverflow,
        // 请求本身非法：重试没用，归到 Provider 侧（下面 is_retryable 会拦住）
        LlmErrorKind::BadRequest => ErrorType::ProviderError,
        LlmErrorKind::Unknown => ErrorType::Unknown,
    }
}

/// Check if an error type is retryable.
///
/// `ProviderError` 不再算可重试：BadRequest 也落在这一类，重试只会原地
/// 复现。真正值得自动重试的（网络抖动 / 5xx / 过载）在 agent 侧已经退避
/// 重试过 3 次了，弹到这里说明那些都没成 —— 由用户决定下一步。
pub fn is_retryable(error_type: &ErrorType) -> bool {
    matches!(
        error_type,
        ErrorType::RateLimit | ErrorType::NetworkError | ErrorType::Unknown
    )
}

fn error_type_label(error_type: &ErrorType) -> (&'static str, Color) {
    match error_type {
        ErrorType::RateLimit => ("Rate Limit", Color::Yellow),
        ErrorType::AuthError => ("Auth Error", Color::Red),
        ErrorType::QuotaError => ("Quota / Billing", Color::Red),
        ErrorType::NetworkError => ("Network Error", Color::Yellow),
        ErrorType::ProviderError => ("Provider Error", Color::Red),
        ErrorType::ContextOverflow => ("Context Overflow", Color::Magenta),
        ErrorType::Unknown => ("Error", Color::Red),
    }
}

/// 给 toast / 状态栏用的短标题。浮层不弹的时候，这是用户唯一会立刻看到的一行。
pub fn error_type_title(error_type: &ErrorType) -> String {
    error_type_label(error_type).0.to_string()
}

/// Render the error overlay
pub fn render_error_overlay(f: &mut Frame, state: &ErrorOverlayState, area: Rect, theme: &Theme) {
    let width = 60.min(area.width.saturating_sub(4));
    let height = 14.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect {
        x,
        y,
        width,
        height,
    };

    f.render_widget(Clear, popup_area);

    let (type_label, type_color) = error_type_label(&state.error_type);

    let mut lines = Vec::new();

    // Title
    lines.push(Line::from(vec![Span::styled(
        format!(" {} ", type_label),
        Style::default()
            .fg(Color::White)
            .bg(type_color)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Error message (truncated)
    let max_msg_len = (width as usize).saturating_sub(4);
    let msg = if state.error_message.len() > max_msg_len {
        format!(
            "{}...",
            &state.error_message[..max_msg_len.saturating_sub(3)]
        )
    } else {
        state.error_message.clone()
    };
    lines.push(Line::from(Span::styled(
        msg,
        Style::default().fg(theme.foreground),
    )));
    lines.push(Line::from(""));

    // Retry info
    if state.retry_count > 0 {
        lines.push(Line::from(Span::styled(
            format!(" {}/{}", state.retry_count, state.max_retries),
            Style::default().fg(theme.secondary),
        )));
        lines.push(Line::from(""));
    }

    // Action buttons
    let actions = [
        (
            ErrorAction::Retry,
            i18n::t("ui.error.retry", "重试", "Retry"),
            "r",
        ),
        (
            ErrorAction::Cancel,
            i18n::t("ui.error.cancel", "取消", "Cancel"),
            "c",
        ),
        (
            ErrorAction::SwitchProvider,
            i18n::t("ui.error.switch_provider", "切换提供商", "Switch Provider"),
            "s",
        ),
    ];

    let mut action_spans = Vec::new();
    for (i, (action, label, key)) in actions.iter().enumerate() {
        if i > 0 {
            action_spans.push(Span::styled("  ", Style::default()));
        }
        let is_selected = state.selected_action == *action;
        let style = if is_selected {
            Style::default()
                .fg(Color::White)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.secondary)
        };
        action_spans.push(Span::styled(format!(" [{}] {} ", key, label), style));
    }
    lines.push(Line::from(action_spans));
    lines.push(Line::from(""));

    // Hint
    lines.push(Line::from(Span::styled(
        i18n::t(
            "ui.error.hint",
            "↑/↓ 导航 · Enter 确认",
            "↑/↓ navigate · Enter confirm",
        ),
        Style::default().fg(theme.comment),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(type_color))
        .title(Span::styled(
            format!(" {} ", i18n::t("ui.error.title", "错误", "Error")),
            Style::default().fg(type_color).add_modifier(Modifier::BOLD),
        ));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, popup_area);
}
