use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// Spinner 帧
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 简单的 ASCII spinner
const ASCII_SPINNER_FRAMES: &[&str] = &["|", "/", "-", "\\"];

/// 获取当前 spinner 帧
pub fn get_spinner_frame(tick: u64) -> &'static str {
    let idx = (tick as usize) % SPINNER_FRAMES.len();
    SPINNER_FRAMES[idx]
}

/// 获取 ASCII spinner 帧
pub fn get_ascii_spinner_frame(tick: u64) -> &'static str {
    let idx = (tick as usize) % ASCII_SPINNER_FRAMES.len();
    ASCII_SPINNER_FRAMES[idx]
}

/// 渲染 spinner
pub fn render_spinner(tick: u64, color: Color) -> Span<'static> {
    Span::styled(
        get_spinner_frame(tick),
        Style::default().fg(color),
    )
}

/// 渲染带文本的 spinner
pub fn render_spinner_with_text(tick: u64, text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!("{} {}", get_spinner_frame(tick), text),
        Style::default().fg(color),
    )
}

/// 渲染加载 spinner
pub fn render_loading_spinner(tick: u64) -> Span<'static> {
    render_spinner(tick, Color::Blue)
}

/// 渲染处理中 spinner — Claude Code 风格随机动词
pub fn render_processing_spinner(tick: u64) -> Span<'static> {
    let verb = crate::ui::components::status_line::random_spinner_verb();
    render_spinner_with_text(tick, &format!("{}...", verb), Color::Rgb(215, 119, 87))
}

/// 渲染思考中 spinner — Claude Code 风格赤陶色
pub fn render_thinking_spinner(tick: u64) -> Span<'static> {
    let verb = crate::ui::components::status_line::random_spinner_verb();
    render_spinner_with_text(tick, &format!("{}...", verb), Color::Rgb(215, 119, 87))
}

/// 渲染带 shimmer 效果的文本
pub fn render_shimmer_text(
    text: &str,
    tick: u64,
    base_color: Color,
    shimmer_color: Color,
    speed: u64,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    
    if len == 0 {
        return vec![Span::raw("")];
    }
    
    let cycle_len = len + 10;
    let pos = (tick / speed) as usize % cycle_len;
    let shimmer_start = if pos >= 10 { pos - 10 } else { 0 };
    let shimmer_end = pos.min(len);
    
    let mut spans = Vec::new();
    let mut current_text = String::new();
    
    for (i, ch) in chars.iter().enumerate() {
        let is_shimmer = i >= shimmer_start && i < shimmer_end;
        
        if is_shimmer {
            // 先输出累积的普通文本
            if !current_text.is_empty() {
                spans.push(Span::styled(
                    current_text.clone(),
                    Style::default().fg(base_color),
                ));
                current_text.clear();
            }
            
            // 输出 shimmer 字符
            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(shimmer_color),
            ));
        } else {
            current_text.push(*ch);
        }
    }
    
    // 输出剩余的普通文本
    if !current_text.is_empty() {
        spans.push(Span::styled(
            current_text,
            Style::default().fg(base_color),
        ));
    }
    
    spans
}

/// 渲染脉冲效果
pub fn render_pulse_color(
    base_color: Color,
    tick: u64,
    speed: u64,
) -> Color {
    let phase = (tick / speed) % 6;
    let intensity = match phase {
        0 => 0.7,
        1 => 0.85,
        2 => 1.0,
        3 => 1.0,
        4 => 0.85,
        _ => 0.7,
    };
    
    match base_color {
        Color::Rgb(r, g, b) => {
            Color::Rgb(
                (r as f64 * intensity) as u8,
                (g as f64 * intensity) as u8,
                (b as f64 * intensity) as u8,
            )
        }
        _ => base_color,
    }
}

/// 渲染带进度的 spinner
pub fn render_progress_spinner(
    tick: u64,
    progress: f32,
    width: usize,
    color: Color,
) -> Span<'static> {
    let filled = (progress * width as f32).round() as usize;
    let empty = width.saturating_sub(filled);
    
    let spinner = get_spinner_frame(tick);
    let bar = format!(
        "[{}{}]",
        "█".repeat(filled),
        "░".repeat(empty)
    );
    
    Span::styled(
        format!("{} {}", spinner, bar),
        Style::default().fg(color),
    )
}
