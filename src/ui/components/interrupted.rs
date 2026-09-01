/// 用户中断显示组件
/// 
/// 对标claude-code-main的InterruptedByUser组件
/// 显示用户中断消息

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// 渲染用户中断消息
/// 
/// 输出格式：
/// - "Interrupted · What should Claude do instead?"
/// - "Interrupted · /issue to report a model issue"
pub fn render_interrupted_by_user(is_ant_user: bool) -> Line<'static> {
    let mut spans = Vec::new();
    
    // "Interrupted"文本
    spans.push(Span::styled(
        "Interrupted",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 分隔符
    spans.push(Span::styled(
        " · ",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 提示文本
    if is_ant_user {
        spans.push(Span::styled(
            "[ANT-ONLY] /issue to report a model issue",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::styled(
            "What should Claude do instead?",
            Style::default().fg(Color::DarkGray),
        ));
    }
    
    Line::from(spans)
}

/// 渲染简洁的用户中断消息（用于状态栏）
pub fn render_interrupted_compact() -> String {
    "Interrupted".to_string()
}

/// 渲染用户中断消息（带自定义提示）
pub fn render_interrupted_with_hint(hint: &str) -> Line<'static> {
    let mut spans = Vec::new();
    
    // "Interrupted"文本
    spans.push(Span::styled(
        "Interrupted",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 分隔符
    spans.push(Span::styled(
        " · ",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 自定义提示
    spans.push(Span::styled(
        hint.to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    
    Line::from(spans)
}

/// 渲染用户中断消息（带操作提示）
pub fn render_interrupted_with_actions(actions: &[&str]) -> Line<'static> {
    let mut spans = Vec::new();
    
    // "Interrupted"文本
    spans.push(Span::styled(
        "Interrupted",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 分隔符
    spans.push(Span::styled(
        " · ",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 操作提示
    let actions_text = actions.join(", ");
    spans.push(Span::styled(
        actions_text,
        Style::default().fg(Color::DarkGray),
    ));
    
    Line::from(spans)
}

/// 渲染用户中断消息（带时间戳）
pub fn render_interrupted_with_timestamp(timestamp: &str) -> Line<'static> {
    let mut spans = Vec::new();
    
    // "Interrupted"文本
    spans.push(Span::styled(
        "Interrupted",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 分隔符
    spans.push(Span::styled(
        " · ",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 时间戳
    spans.push(Span::styled(
        timestamp.to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    
    Line::from(spans)
}

/// 渲染用户中断消息（带原因）
pub fn render_interrupted_with_reason(reason: &str) -> Line<'static> {
    let mut spans = Vec::new();
    
    // "Interrupted"文本
    spans.push(Span::styled(
        "Interrupted",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 分隔符
    spans.push(Span::styled(
        " · ",
        Style::default().fg(Color::DarkGray),
    ));
    
    // 原因
    spans.push(Span::styled(
        reason.to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    
    Line::from(spans)
}
