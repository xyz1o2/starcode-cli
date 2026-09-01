/// PR徽章组件
/// 
/// 对标claude-code-main的PrBadge组件
/// 显示PR状态和链接

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// PR审查状态
#[derive(Debug, Clone, PartialEq)]
pub enum PrReviewState {
    /// 已批准
    Approved,
    /// 请求更改
    ChangesRequested,
    /// 待审查
    Pending,
    /// 已合并
    Merged,
}

/// PR信息
#[derive(Debug, Clone)]
pub struct PrInfo {
    /// PR编号
    pub number: u32,
    /// PR URL
    pub url: String,
    /// 审查状态
    pub review_state: Option<PrReviewState>,
    /// 是否加粗
    pub bold: bool,
}

/// 获取PR状态颜色
fn get_pr_status_color(state: &Option<PrReviewState>) -> Option<Color> {
    match state {
        Some(PrReviewState::Approved) => Some(Color::Green),
        Some(PrReviewState::ChangesRequested) => Some(Color::Red),
        Some(PrReviewState::Pending) => Some(Color::Yellow),
        Some(PrReviewState::Merged) => Some(Color::Magenta),
        None => None,
    }
}

/// 渲染PR徽章
/// 
/// 输出格式：
/// - PR #123 (approved)
/// - PR #456 (changes requested)
/// - PR #789 (pending)
/// - PR #101 (merged)
pub fn render_pr_badge(pr: &PrInfo) -> Line<'static> {
    let status_color = get_pr_status_color(&pr.review_state);
    let mut spans = Vec::new();
    
    // PR标签
    spans.push(Span::styled(
        "PR",
        Style::default().fg(Color::DarkGray),
    ));
    
    // PR编号
    let number_style = if let Some(color) = status_color {
        Style::default().fg(color).add_modifier(if pr.bold { Modifier::BOLD } else { Modifier::empty() })
    } else {
        Style::default().fg(Color::DarkGray).add_modifier(if pr.bold { Modifier::BOLD } else { Modifier::empty() })
    };
    
    spans.push(Span::styled(
        format!(" #{}", pr.number),
        number_style,
    ));
    
    Line::from(spans)
}

/// 渲染PR状态标签
pub fn render_pr_status_label(state: &PrReviewState) -> &'static str {
    match state {
        PrReviewState::Approved => "approved",
        PrReviewState::ChangesRequested => "changes requested",
        PrReviewState::Pending => "pending",
        PrReviewState::Merged => "merged",
    }
}

/// 渲染简洁的PR状态（用于状态栏）
pub fn render_pr_compact(pr: &PrInfo) -> String {
    let status = match &pr.review_state {
        Some(state) => format!(" ({})", render_pr_status_label(state)),
        None => String::new(),
    };
    
    format!("PR #{}{}", pr.number, status)
}
