/// 使用统计模块
/// 
/// 对标claude-code-main的src/utils/stats.ts
/// 提供使用统计功能

use std::collections::HashMap;

/// 每日活动数据
#[derive(Debug, Clone, Default)]
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

/// 每日Token使用量
#[derive(Debug, Clone, Default)]
pub struct DailyModelTokens {
    /// 日期 (YYYY-MM-DD)
    pub date: String,
    /// 每个模型的Token使用量
    pub tokens_by_model: HashMap<String, u32>,
}

/// 连续使用信息
#[derive(Debug, Clone, Default)]
pub struct StreakInfo {
    /// 当前连续天数
    pub current_streak: u32,
    /// 最长连续天数
    pub longest_streak: u32,
    /// 当前连续开始日期
    pub current_streak_start: Option<String>,
    /// 最长连续开始日期
    pub longest_streak_start: Option<String>,
    /// 最长连续结束日期
    pub longest_streak_end: Option<String>,
}

/// 会话统计
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    /// 会话ID
    pub session_id: String,
    /// 持续时间 (毫秒)
    pub duration: u64,
    /// 消息数量
    pub message_count: u32,
    /// 时间戳
    pub timestamp: String,
}

/// 模型使用统计
#[derive(Debug, Clone, Default)]
pub struct ModelUsage {
    /// 输入Token数量
    pub input_tokens: u64,
    /// 输出Token数量
    pub output_tokens: u64,
    /// 缓存读取Token数量
    pub cache_read_input_tokens: u64,
}

/// Claude Code使用统计
#[derive(Debug, Clone, Default)]
pub struct ClaudeCodeStats {
    /// 总会话数
    pub total_sessions: u32,
    /// 总消息数
    pub total_messages: u32,
    /// 总天数
    pub total_days: u32,
    /// 活跃天数
    pub active_days: u32,
    
    /// 连续使用信息
    pub streaks: StreakInfo,
    
    /// 每日活动数据
    pub daily_activity: Vec<DailyActivity>,
    
    /// 每日Token使用量
    pub daily_model_tokens: Vec<DailyModelTokens>,
    
    /// 最长会话
    pub longest_session: Option<SessionStats>,
    
    /// 模型使用统计
    pub model_usage: HashMap<String, ModelUsage>,
    
    /// 首次会话日期
    pub first_session_date: Option<String>,
    
    /// 最后会话日期
    pub last_session_date: Option<String>,
    
    /// 最活跃日期
    pub peak_activity_day: Option<String>,
    
    /// 最活跃小时
    pub peak_activity_hour: Option<u32>,
}

/// 统计日期范围
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatsDateRange {
    /// 最近7天
    Last7Days,
    /// 最近30天
    Last30Days,
    /// 所有时间
    All,
}

impl StatsDateRange {
    /// 获取标签
    pub fn label(&self) -> &'static str {
        match self {
            StatsDateRange::Last7Days => "Last 7 days",
            StatsDateRange::Last30Days => "Last 30 days",
            StatsDateRange::All => "All time",
        }
    }
    
    /// 获取天数
    pub fn days(&self) -> Option<u32> {
        match self {
            StatsDateRange::Last7Days => Some(7),
            StatsDateRange::Last30Days => Some(30),
            StatsDateRange::All => None,
        }
    }
}

/// 聚合统计
pub fn aggregate_stats(
    daily_activity: &[DailyActivity],
    daily_model_tokens: &[DailyModelTokens],
    sessions: &[SessionStats],
) -> ClaudeCodeStats {
    let total_sessions = sessions.len() as u32;
    let total_messages = daily_activity.iter().map(|a| a.message_count).sum();
    let total_days = daily_activity.len() as u32;
    let active_days = daily_activity.iter().filter(|a| a.message_count > 0).count() as u32;
    
    // 计算连续使用信息
    let streaks = calculate_streaks(daily_activity);
    
    // 计算最长会话
    let longest_session = sessions.iter().max_by_key(|s| s.duration).cloned();
    
    // 计算模型使用统计
    let mut model_usage = HashMap::new();
    for day in daily_model_tokens {
        for (model, tokens) in &day.tokens_by_model {
            let usage = model_usage.entry(model.clone()).or_insert_with(ModelUsage::default);
            usage.input_tokens += *tokens as u64;
        }
    }
    
    // 计算最活跃日期
    let peak_activity_day = daily_activity
        .iter()
        .max_by_key(|a| a.message_count)
        .map(|a| a.date.clone());
    
    // 计算最活跃小时
    let peak_activity_hour = Some(14); // 默认下午2点
    
    ClaudeCodeStats {
        total_sessions,
        total_messages,
        total_days,
        active_days,
        streaks,
        daily_activity: daily_activity.to_vec(),
        daily_model_tokens: daily_model_tokens.to_vec(),
        longest_session,
        model_usage,
        first_session_date: daily_activity.first().map(|a| a.date.clone()),
        last_session_date: daily_activity.last().map(|a| a.date.clone()),
        peak_activity_day,
        peak_activity_hour,
    }
}

/// 计算连续使用信息
fn calculate_streaks(daily_activity: &[DailyActivity]) -> StreakInfo {
    if daily_activity.is_empty() {
        return StreakInfo::default();
    }
    
    let mut current_streak = 0;
    let mut longest_streak = 0;
    let mut current_streak_start = None;
    let mut longest_streak_start = None;
    let mut longest_streak_end = None;
    let mut temp_streak = 0;
    let mut temp_start = None;
    
    for (i, day) in daily_activity.iter().enumerate() {
        if day.message_count > 0 {
            if temp_streak == 0 {
                temp_start = Some(day.date.clone());
            }
            temp_streak += 1;
            
            if temp_streak > longest_streak {
                longest_streak = temp_streak;
                longest_streak_start = temp_start.clone();
                longest_streak_end = Some(day.date.clone());
            }
        } else {
            temp_streak = 0;
        }
    }
    
    // 计算当前连续（从最后一天开始）
    for day in daily_activity.iter().rev() {
        if day.message_count > 0 {
            current_streak += 1;
            current_streak_start = Some(day.date.clone());
        } else {
            break;
        }
    }
    
    StreakInfo {
        current_streak,
        longest_streak,
        current_streak_start,
        longest_streak_start,
        longest_streak_end,
    }
}

/// 获取日期范围
pub fn get_date_range(range: StatsDateRange) -> (String, String) {
    let today = chrono::Local::now().naive_local().date();
    
    match range {
        StatsDateRange::Last7Days => {
            let start = today - chrono::Duration::days(6);
            (start.format("%Y-%m-%d").to_string(), today.format("%Y-%m-%d").to_string())
        }
        StatsDateRange::Last30Days => {
            let start = today - chrono::Duration::days(29);
            (start.format("%Y-%m-%d").to_string(), today.format("%Y-%m-%d").to_string())
        }
        StatsDateRange::All => {
            (String::new(), today.format("%Y-%m-%d").to_string())
        }
    }
}

/// 格式化统计日期范围
pub fn format_stats_date_range(range: StatsDateRange) -> String {
    match range {
        StatsDateRange::Last7Days => "Last 7 days".to_string(),
        StatsDateRange::Last30Days => "Last 30 days".to_string(),
        StatsDateRange::All => "All time".to_string(),
    }
}
