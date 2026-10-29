/// Error overlay — provider 错误走**统一弹窗外壳**（`common::modal_shell`，
/// 与 /mcp /market /plugin 同一套骨架）：几何、清屏、边框、标题、底部按键条
/// 都归外壳管，这里只产出内容行。之前自己手搓了一份居中矩形 + 徽章标题 +
/// 内联 `[r] 按钮`，样式和别的弹窗对不上，改一处主题要记两套写法。
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    Frame,
};

use super::common::modal_shell::{
    footer_hints, modal_shell, render_body, row_style, with_footer, ModalSpec,
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

/// 渲染错误浮层 —— 统一外壳 + 标准选项列表。
///
/// 三个动作的交互没有变：↑/↓/Enter 走选中项，`r` / `c` / `s` 仍然直开直执行；
/// 变的只是外观：选项行用和确认卡、MCP 列表一致的 `❯` 高亮列表，
/// 按键说明收进外壳统一管理的 footer。
pub fn render_error_overlay(f: &mut Frame, state: &ErrorOverlayState, area: Rect, theme: &Theme) {
    let (type_label, type_color) = error_type_label(&state.error_type);

    let inner = modal_shell(
        f,
        area,
        &ModalSpec {
            title: i18n::t("ui.error.title", "错误", "Error"),
            accent: type_color,
            size: Some((ERROR_DIALOG_WIDTH, ERROR_DIALOG_HEIGHT)),
            ..ModalSpec::default()
        },
    );
    let (body, footer) = with_footer(inner);

    let mut lines = Vec::new();

    // 分类行：外壳标题之外，第一眼要看到错误"是什么类型"。
    lines.push(Line::from(Span::styled(
        format!(" ⚠ {}", type_label),
        Style::default()
            .fg(type_color)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // 错误消息。按**字符**折行（provider 的错误里混中文，按字节切会落在
    // 非 UTF-8 边界上 panic）——消息全文已经在 chat_history 里，浮层里
    // 折不完的部分折行计数收尾，不截掉信息。
    let wrap_width = body.width.saturating_sub(4) as usize;
    let show = wrap_with_cap(&state.error_message, wrap_width, MESSAGE_MAX_LINES);
    for text in show {
        lines.push(Line::from(Span::raw(text)));
    }

    // Retry info
    if state.retry_count > 0 {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(" {}/{}", state.retry_count, state.max_retries),
            Style::default().fg(theme.secondary),
        )));
    }

    // Action rows —— 与确认卡同一套 `❯ N. label` 头部；快捷键显示在行首。
    lines.push(Line::from(""));
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
    for (action, label, key) in actions {
        let selected = state.selected_action == action;
        // 统一列表行高亮：与 MCP / market 列表同一种选中样式（黑底青字）。
        let row = row_style(0, usize::from(selected));
        let arrow = Span::styled(
            if selected { " ❯ " } else { "   " },
            Style::default().fg(theme.primary),
        );
        let key_span = Span::styled(format!("{} ", key), row);
        let text_span = Span::styled(label, row);
        lines.push(Line::from(vec![arrow, key_span, text_span]));
    }

    render_body(f, body, lines);

    // 外壳统一的底部按键条；`Esc` 的行为同 `c`（关浮层，不放弃重试机会）。
    let hints: &[(&str, &str)] = &[
        ("↑↓", "select"),
        ("Enter", "confirm"),
        ("r", "retry"),
        ("c", "cancel"),
        ("s", "provider"),
        ("Esc", "close"),
    ];
    render_body(f, footer, vec![footer_hints(hints)]);
}

/// 浮层的固定格子尺寸：错误确认弹窗要小而稳，
/// 用百分比几何会在宽屏上被拉成横幅。
const ERROR_DIALOG_WIDTH: u16 = 62;
const ERROR_DIALOG_HEIGHT: u16 = 13;
/// 消息超出多少行后收为"还有 N 行"——浮层高度被这两项锁着。
const MESSAGE_MAX_LINES: usize = 3;

/// 按字符数折行（近似 display width），最多“cap 行正文 + 1 行计数”。
fn wrap_with_cap(text: &str, width: usize, cap: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            let word = if word.chars().count() > width {
                word.chars().take(width).collect::<String>()
            } else {
                word.to_string()
            };
            if current.is_empty() {
                current = word;
            } else if current.chars().count() + word.chars().count() >= width {
                out.push(current);
                current = word;
            } else {
                current.push(' ');
                current.push_str(&word);
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }

    if out.len() > cap {
        let hidden = out.len() - cap;
        out.truncate(cap);
        out.push(format!("  … +{} lines (full text in chat)", hidden));
    }
    out
}
