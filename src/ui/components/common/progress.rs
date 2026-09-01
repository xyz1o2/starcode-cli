use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// 进度条配置
pub struct ProgressBarConfig {
    pub width: usize,
    pub filled_char: char,
    pub empty_char: char,
    pub show_percentage: bool,
    pub color: Color,
    pub background_color: Color,
}

impl Default for ProgressBarConfig {
    fn default() -> Self {
        Self {
            width: 20,
            filled_char: '█',
            empty_char: '░',
            show_percentage: true,
            color: Color::Blue,
            background_color: Color::DarkGray,
        }
    }
}

/// 渲染进度条
pub fn render_progress_bar(progress: f32, config: &ProgressBarConfig) -> Span<'static> {
    let progress = progress.clamp(0.0, 1.0);
    let filled = (progress * config.width as f32).round() as usize;
    let empty = config.width.saturating_sub(filled);

    let bar = format!(
        "{}{}",
        config.filled_char.to_string().repeat(filled),
        config.empty_char.to_string().repeat(empty)
    );

    if config.show_percentage {
        let percentage = (progress * 100.0).round() as u32;
        Span::styled(
            format!("{} {}%", bar, percentage),
            Style::default().fg(config.color),
        )
    } else {
        Span::styled(bar, Style::default().fg(config.color))
    }
}

/// 渲染简单的进度条
pub fn render_simple_progress_bar(progress: f32, width: usize) -> Span<'static> {
    let config = ProgressBarConfig {
        width,
        ..Default::default()
    };
    render_progress_bar(progress, &config)
}

/// 渲染带标签的进度条
pub fn render_labeled_progress_bar(label: &str, progress: f32, width: usize) -> Line<'static> {
    let bar = render_simple_progress_bar(progress, width);

    Line::from(vec![
        Span::styled(format!("{}: ", label), Style::default().fg(Color::White)),
        bar,
    ])
}

/// 渲染多阶段进度条
pub fn render_multi_stage_progress(
    stages: &[(&str, f32)], // (label, progress)
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for (label, progress) in stages {
        lines.push(render_labeled_progress_bar(label, *progress, width));
    }

    lines
}

/// 渲染步骤进度
pub fn render_step_progress(current: usize, total: usize, label: &str) -> Line<'static> {
    let progress = if total > 0 {
        current as f32 / total as f32
    } else {
        0.0
    };

    Line::from(vec![
        Span::styled(
            format!("[{}/{}] ", current, total),
            Style::default().fg(Color::Blue),
        ),
        Span::styled(label.to_string(), Style::default().fg(Color::White)),
        Span::styled(
            format!(" ({}%)", (progress * 100.0).round() as u32),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

/// 渲染带时间的进度条
pub fn render_timed_progress_bar(
    progress: f32,
    elapsed: std::time::Duration,
    width: usize,
) -> Line<'static> {
    let bar = render_simple_progress_bar(progress, width);
    let elapsed_str = format_elapsed(elapsed);

    Line::from(vec![
        bar,
        Span::styled(
            format!(" {}", elapsed_str),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

/// 格式化时间
fn format_elapsed(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();

    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// 渲染环形进度指示器
pub fn render_circular_progress(
    progress: f32,
    radius: usize,
    filled_char: char,
    empty_char: char,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let progress = progress.clamp(0.0, 1.0);

    // 简化的环形进度（使用字符模拟）
    let filled = (progress * radius as f32).round() as usize;
    let empty = radius.saturating_sub(filled);

    let line = Line::from(vec![
        Span::styled(
            filled_char.to_string().repeat(filled),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            empty_char.to_string().repeat(empty),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    lines.push(line);
    lines
}
