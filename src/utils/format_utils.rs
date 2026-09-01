/// 格式化工具
///
/// 对标claude-code-main的src/utils/format.ts

/// 格式化文件大小
pub fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// 格式化持续时间
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60 * 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 60 * 60 * 1000 {
        let mins = ms / (60 * 1000);
        let secs = (ms % (60 * 1000)) / 1000;
        format!("{}m {}s", mins, secs)
    } else {
        let hours = ms / (60 * 60 * 1000);
        let mins = (ms % (60 * 60 * 1000)) / (60 * 1000);
        format!("{}h {}m", hours, mins)
    }
}

/// 格式化数字
pub fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();

    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }

    result.chars().rev().collect()
}

/// 格式化百分比
pub fn format_percentage(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

/// 格式化token数
pub fn format_tokens(tokens: u32) -> String {
    if tokens < 1000 {
        format!("{}", tokens)
    } else if tokens < 1000000 {
        format!("{:.1}K", tokens as f64 / 1000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1000000.0)
    }
}

/// 格式化成本
pub fn format_cost(cost: f64) -> String {
    if cost < 0.01 {
        format!("${:.4}", cost)
    } else if cost < 1.0 {
        format!("${:.2}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

/// 格式化时间戳
pub fn format_timestamp(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// 格式化日期
pub fn format_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// 格式化时间
pub fn format_time(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

/// 格式化列表
pub fn format_list(items: &[String], separator: &str) -> String {
    items.join(separator)
}

/// 格式化表格
pub fn format_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut result = String::new();

    // 计算列宽
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    // 打印表头
    for (i, header) in headers.iter().enumerate() {
        if i > 0 {
            result.push_str(" | ");
        }
        result.push_str(&pad_right(header, widths[i], ' '));
    }
    result.push('\n');

    // 打印分隔线
    for (i, width) in widths.iter().enumerate() {
        if i > 0 {
            result.push_str("-+-");
        }
        result.push_str(&repeat("-", *width));
    }
    result.push('\n');

    // 打印数据行
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                result.push_str(" | ");
            }
            result.push_str(&pad_right(cell, widths[i], ' '));
        }
        result.push('\n');
    }

    result
}

fn pad_right(s: &str, width: usize, fill: char) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        format!("{}{}", s, fill.to_string().repeat(width - s.len()))
    }
}

fn repeat(s: &str, n: usize) -> String {
    s.repeat(n)
}
