/// 内存使用指示器组件
/// 
/// 对标claude-code-main的MemoryUsageIndicator组件
/// 显示内存使用情况

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// 内存使用状态
#[derive(Debug, Clone, PartialEq)]
pub enum MemoryStatus {
    /// 正常
    Normal,
    /// 警告
    Warning,
    /// 危险
    Critical,
}

/// 内存使用信息
#[derive(Debug, Clone)]
pub struct MemoryUsage {
    /// 堆使用量 (bytes)
    pub heap_used: u64,
    /// 堆总量 (bytes)
    pub heap_total: u64,
    /// 外部内存使用量 (bytes)
    pub external: u64,
    /// 状态
    pub status: MemoryStatus,
}

/// 格式化文件大小
fn format_file_size(size: u64) -> String {
    if size < 1024 {
        return format!("{} bytes", size);
    }
    
    let kb = size as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{:.1}KB", kb);
    }
    
    let mb = kb / 1024.0;
    if mb < 1024.0 {
        return format!("{:.1}MB", mb);
    }
    
    let gb = mb / 1024.0;
    format!("{:.1}GB", gb)
}

/// 计算内存使用百分比
fn calculate_usage_percentage(used: u64, total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (used as f64 / total as f64) * 100.0
}

/// 渲染内存使用指示器
/// 
/// 输出格式：
/// - Normal: "Memory: 45.2MB (22.5%)"
/// - Warning: "⚠ Memory: 1.2GB (60.0%)"
/// - Critical: "✗ Memory: 1.8GB (90.0%)"
pub fn render_memory_usage_indicator(usage: &MemoryUsage) -> Option<Line<'static>> {
    // 只在警告或危险状态显示
    if usage.status == MemoryStatus::Normal {
        return None;
    }
    
    let percentage = calculate_usage_percentage(usage.heap_used, usage.heap_total);
    let formatted_size = format_file_size(usage.heap_used);
    
    let (icon, color) = match usage.status {
        MemoryStatus::Warning => ("⚠", Color::Yellow),
        MemoryStatus::Critical => ("✗", Color::Red),
        MemoryStatus::Normal => ("", Color::White),
    };
    
    let mut spans = Vec::new();
    
    // 图标
    if !icon.is_empty() {
        spans.push(Span::styled(
            format!("{} ", icon),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    
    // 内存使用信息
    spans.push(Span::styled(
        format!("Memory: {} ({:.1}%)", formatted_size, percentage),
        Style::default().fg(color),
    ));
    
    Some(Line::from(spans))
}

/// 渲染简洁的内存使用（用于状态栏）
pub fn render_memory_usage_compact(usage: &MemoryUsage) -> Option<String> {
    if usage.status == MemoryStatus::Normal {
        return None;
    }
    
    let percentage = calculate_usage_percentage(usage.heap_used, usage.heap_total);
    let formatted_size = format_file_size(usage.heap_used);
    
    let icon = match usage.status {
        MemoryStatus::Warning => "⚠",
        MemoryStatus::Critical => "✗",
        MemoryStatus::Normal => "",
    };
    
    Some(format!("{} Memory: {} ({:.1}%)", icon, formatted_size, percentage))
}

/// 渲染内存使用条形图
pub fn render_memory_usage_bar(usage: &MemoryUsage, width: usize) -> Line<'static> {
    let percentage = calculate_usage_percentage(usage.heap_used, usage.heap_total);
    let filled = (percentage / 100.0 * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    
    let color = match usage.status {
        MemoryStatus::Normal => Color::Green,
        MemoryStatus::Warning => Color::Yellow,
        MemoryStatus::Critical => Color::Red,
    };
    
    let bar = format!(
        "[{}{}]",
        "█".repeat(filled),
        "░".repeat(empty)
    );
    
    Line::from(vec![
        Span::styled(bar, Style::default().fg(color)),
        Span::styled(
            format!(" {:.1}%", percentage),
            Style::default().fg(Color::White),
        ),
    ])
}

/// 创建内存使用信息
pub fn create_memory_usage(heap_used: u64, heap_total: u64, external: u64) -> MemoryUsage {
    let percentage = calculate_usage_percentage(heap_used, heap_total);
    
    let status = if percentage >= 90.0 {
        MemoryStatus::Critical
    } else if percentage >= 75.0 {
        MemoryStatus::Warning
    } else {
        MemoryStatus::Normal
    };
    
    MemoryUsage {
        heap_used,
        heap_total,
        external,
        status,
    }
}
