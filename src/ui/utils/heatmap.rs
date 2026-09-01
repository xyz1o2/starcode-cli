/// 热力图生成器
/// 
/// 对标claude-code-main的src/utils/heatmap.ts
/// 生成GitHub风格的活动热力图

use std::collections::HashMap;

/// 每日活动数据
#[derive(Debug, Clone)]
pub struct DailyActivity {
    /// 日期 (YYYY-MM-DD)
    pub date: String,
    /// 消息数量
    pub message_count: u32,
    /// 会话数量
    pub session_count: u32,
    /// 工具调用数量
    pub tool_call_count: u32,
}

/// 百分位数
#[derive(Debug, Clone)]
struct Percentiles {
    p25: f64,
    p50: f64,
    p75: f64,
}

/// 计算百分位数
fn calculate_percentiles(activities: &[DailyActivity]) -> Option<Percentiles> {
    let mut counts: Vec<f64> = activities
        .iter()
        .map(|a| a.message_count as f64)
        .filter(|c| *c > 0.0)
        .collect();
    
    if counts.is_empty() {
        return None;
    }
    
    counts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    
    let len = counts.len();
    Some(Percentiles {
        p25: counts[len * 25 / 100],
        p50: counts[len * 50 / 100],
        p75: counts[len * 75 / 100],
    })
}

/// 获取强度等级 (0-4)
fn get_intensity_level(count: u32, percentiles: &Option<Percentiles>) -> usize {
    if count == 0 {
        return 0;
    }
    
    match percentiles {
        Some(p) => {
            let count = count as f64;
            if count <= p.p25 {
                1
            } else if count <= p.p50 {
                2
            } else if count <= p.p75 {
                3
            } else {
                4
            }
        }
        None => 1,
    }
}

/// 热力图块字符
const HEATMAP_CHARS: &[&str] = &["░", "▒", "▓", "█", "█"];

/// 生成热力图
/// 
/// 输出格式：
/// ```
/// Mon ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
/// Tue ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
/// Wed ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
/// Thu ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
/// Fri ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
/// Sat ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
/// Sun ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
///     Jan  Feb  Mar  Apr  May  Jun  Jul  Aug  Sep  Oct  Nov  Dec
/// ```
pub fn generate_heatmap(activities: &[DailyActivity], terminal_width: Option<usize>) -> String {
    let terminal_width = terminal_width.unwrap_or(80);
    let day_label_width = 4;
    let available_width = terminal_width.saturating_sub(day_label_width);
    let width = available_width.min(52).max(10);
    
    // 构建活动映射
    let mut activity_map: HashMap<String, &DailyActivity> = HashMap::new();
    for activity in activities {
        activity_map.insert(activity.date.clone(), activity);
    }
    
    // 计算百分位数
    let percentiles = calculate_percentiles(activities);
    
    // 计算日期范围
    let today = chrono::Local::now().naive_local().date();
    let current_week_start = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    let start_date = current_week_start - chrono::Duration::weeks((width - 1) as i64);
    
    // 生成网格
    let mut grid: Vec<Vec<String>> = vec![vec![String::new(); width]; 7];
    let mut month_starts: Vec<(u32, usize)> = Vec::new();
    let mut last_month = 0;
    
    let mut current_date = start_date;
    for week in 0..width {
        for day in 0..7 {
            // 不显示未来日期
            if current_date > today {
                grid[day][week] = " ".to_string();
                current_date += chrono::Duration::days(1);
                continue;
            }
            
            let date_str = current_date.format("%Y-%m-%d").to_string();
            let activity = activity_map.get(&date_str);
            
            // 跟踪月份变化
            if day == 0 {
                let month = current_date.month();
                if month != last_month {
                    month_starts.push((month, week));
                    last_month = month;
                }
            }
            
            // 获取强度等级
            let level = activity
                .map(|a| get_intensity_level(a.message_count, &percentiles))
                .unwrap_or(0);
            
            grid[day][week] = HEATMAP_CHARS[level].to_string();
            
            current_date += chrono::Duration::days(1);
        }
    }
    
    // 生成输出
    let day_labels = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut lines = Vec::new();
    
    for (day_idx, label) in day_labels.iter().enumerate() {
        let mut line = format!("{:>3} ", label);
        for week in 0..width {
            line.push_str(&grid[day_idx][week]);
        }
        lines.push(line);
    }
    
    // 生成月份标签
    let month_names = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", 
                       "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut month_line = "    ".to_string();
    let mut last_week = 0;
    
    for (month, week) in &month_starts {
        let spaces = week.saturating_sub(last_week);
        month_line.push_str(&" ".repeat(spaces));
        month_line.push_str(month_names[(month - 1) as usize]);
        last_week = week + 3;
    }
    
    lines.push(month_line);
    
    lines.join("\n")
}

/// 生成简洁的热力图（用于状态栏）
pub fn generate_heatmap_compact(activities: &[DailyActivity]) -> String {
    let percentiles = calculate_percentiles(activities);
    
    // 获取最近7天的活动
    let today = chrono::Local::now().naive_local().date();
    let mut recent_activities = Vec::new();
    
    for i in 0..7 {
        let date = today - chrono::Duration::days(i);
        let date_str = date.format("%Y-%m-%d").to_string();
        let count = activities
            .iter()
            .find(|a| a.date == date_str)
            .map(|a| a.message_count)
            .unwrap_or(0);
        recent_activities.push(count);
    }
    
    recent_activities.reverse();
    
    // 生成简洁的热力图
    let mut line = String::new();
    for count in recent_activities {
        let level = get_intensity_level(count, &percentiles);
        line.push_str(HEATMAP_CHARS[level]);
    }
    
    line
}
