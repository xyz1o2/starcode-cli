/// Agent进度行组件
/// 
/// 对标claude-code-main的AgentProgressLine组件
/// 显示Agent执行进度

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Agent进度信息
#[derive(Debug, Clone)]
pub struct AgentProgress {
    /// Agent类型
    pub agent_type: String,
    /// Agent描述
    pub description: Option<String>,
    /// Agent名称
    pub name: Option<String>,
    /// 任务描述
    pub task_description: Option<String>,
    /// 工具使用次数
    pub tool_use_count: u32,
    /// Token使用量
    pub tokens: Option<u32>,
    /// 是否完成
    pub is_resolved: bool,
    /// 是否错误
    pub is_error: bool,
    /// 是否异步
    pub is_async: bool,
    /// 最后工具信息
    pub last_tool_info: Option<String>,
    /// 是否隐藏类型
    pub hide_type: bool,
}

/// 格式化token数量
fn format_tokens(count: u32) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}k", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}

/// 渲染Agent进度行
/// 
/// 输出格式：
/// ```
/// ├─ AgentType (Description)
/// │   ├─ Initializing…
/// │   └─ Done · 12 tools · 45k tokens
/// ```
pub fn render_agent_progress_line(
    progress: &AgentProgress,
    is_last: bool,
    theme_color: Color,
    description_color: Option<Color>,
) -> Line<'static> {
    let tree_char = if is_last { "└─" } else { "├─" };
    let is_backgrounded = progress.is_async && progress.is_resolved;
    
    // 确定状态文本
    let status_text = if !progress.is_resolved {
        progress.last_tool_info.as_deref().unwrap_or("Initializing…")
    } else if is_backgrounded {
        progress.task_description.as_deref().unwrap_or("Running in the background")
    } else {
        "Done"
    };
    
    let mut spans = Vec::new();
    
    // 树形字符
    spans.push(Span::styled(
        format!("   {} ", tree_char),
        Style::default().fg(Color::DarkGray),
    ));
    
    // Agent类型/名称
    if progress.hide_type {
        let display_name = progress.name.as_deref()
            .or(progress.description.as_deref())
            .unwrap_or(&progress.agent_type);
        spans.push(Span::styled(
            display_name.to_string(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ));
        
        if let (Some(name), Some(desc)) = (&progress.name, &progress.description) {
            spans.push(Span::styled(
                format!(": {}", desc),
                Style::default().fg(Color::DarkGray),
            ));
        }
    } else {
        spans.push(Span::styled(
            progress.agent_type.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
        
        if let Some(desc) = &progress.description {
            spans.push(Span::styled(
                format!(" ({})", desc),
                Style::default().fg(Color::DarkGray),
            ));
        }
    }
    
    // 状态文本
    let status_color = if progress.is_resolved {
        if progress.is_error {
            Color::Red
        } else {
            Color::Green
        }
    } else {
        Color::Yellow
    };
    
    spans.push(Span::styled(
        format!(" {}", status_text),
        Style::default().fg(status_color),
    ));
    
    // 工具使用次数
    if progress.tool_use_count > 0 {
        spans.push(Span::styled(
            format!(" · {} tools", progress.tool_use_count),
            Style::default().fg(Color::DarkGray),
        ));
    }
    
    // Token使用量
    if let Some(tokens) = progress.tokens {
        spans.push(Span::styled(
            format!(" · {} tokens", format_tokens(tokens)),
            Style::default().fg(Color::DarkGray),
        ));
    }
    
    Line::from(spans)
}

/// 渲染Agent进度树
/// 
/// 显示多个Agent的进度树
pub fn render_agent_progress_tree(
    progresses: &[AgentProgress],
    theme_color: Color,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    for (i, progress) in progresses.iter().enumerate() {
        let is_last = i == progresses.len() - 1;
        lines.push(render_agent_progress_line(
            progress,
            is_last,
            theme_color,
            None,
        ));
    }
    
    lines
}

/// 渲染简洁的Agent进度（用于状态栏）
pub fn render_agent_progress_compact(progresses: &[AgentProgress]) -> String {
    if progresses.is_empty() {
        return String::new();
    }
    
    let running = progresses.iter().filter(|p| !p.is_resolved).count();
    let completed = progresses.iter().filter(|p| p.is_resolved && !p.is_error).count();
    let failed = progresses.iter().filter(|p| p.is_resolved && p.is_error).count();
    
    let mut parts = Vec::new();
    
    if running > 0 {
        parts.push(format!("{} running", running));
    }
    if completed > 0 {
        parts.push(format!("{} done", completed));
    }
    if failed > 0 {
        parts.push(format!("{} failed", failed));
    }
    
    parts.join(", ")
}
