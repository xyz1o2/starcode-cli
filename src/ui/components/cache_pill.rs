/// 缓存命中率显示组件
/// 
/// 对标claude-code-main的CachePill组件
/// 显示缓存命中率和TTL倒计时

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// 缓存统计状态
#[derive(Debug, Clone)]
pub struct CacheStatsState {
    /// 最后重置时间
    pub last_reset_at: Option<std::time::Instant>,
    /// 最后命中率
    pub last_hit_rate: Option<f64>,
    /// 累计命中次数
    pub hits: u64,
    /// 累计未命中次数
    pub misses: u64,
}

impl Default for CacheStatsState {
    fn default() -> Self {
        Self {
            last_reset_at: None,
            last_hit_rate: None,
            hits: 0,
            misses: 0,
        }
    }
}

impl CacheStatsState {
    /// 更新命中率
    pub fn update_hit_rate(&mut self, hit: bool) {
        if hit {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        
        let total = self.hits + self.misses;
        if total > 0 {
            self.last_hit_rate = Some(self.hits as f64 / total as f64 * 100.0);
        }
    }

    /// 重置统计
    pub fn reset(&mut self) {
        self.last_reset_at = Some(std::time::Instant::now());
        self.hits = 0;
        self.misses = 0;
        self.last_hit_rate = None;
    }

    /// 获取命中率百分比
    pub fn hit_rate_percent(&self) -> Option<f64> {
        self.last_hit_rate
    }

    /// 获取剩余TTL（秒）
    pub fn remaining_ttl_secs(&self) -> Option<u64> {
        self.last_reset_at.map(|t| {
            let elapsed = t.elapsed().as_secs();
            if elapsed >= 3600 {
                0
            } else {
                3600 - elapsed
            }
        })
    }
}

/// 格式化倒计时
fn format_countdown(remaining_secs: u64) -> String {
    if remaining_secs == 0 {
        return "exp".to_string();
    }
    let mins = remaining_secs / 60;
    let secs = remaining_secs % 60;
    format!("{:02}:{:02}", mins, secs)
}

/// 渲染缓存命中率显示
/// 
/// 输出格式: Cache 85% 45:30
pub fn render_cache_pill(stats: &CacheStatsState) -> Line<'static> {
    let hit_rate_text = match stats.hit_rate_percent() {
        Some(rate) => format!("{:.0}%", rate),
        None => "--%".to_string(),
    };

    let countdown_text = match stats.remaining_ttl_secs() {
        Some(secs) => format_countdown(secs),
        None => "--:--".to_string(),
    };

    // 颜色逻辑
    let hit_rate_color = match stats.hit_rate_percent() {
        Some(rate) if rate >= 50.0 => Color::Rgb(80, 220, 100),  // Green
        Some(_) => Color::DarkGray,
        None => Color::DarkGray,
    };

    // 倒计时颜色
    let elapsed = stats.last_reset_at
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);
    let elapsed_min = elapsed as f64 / 60.0;
    
    let timer_color = if elapsed >= 3600 {
        Color::DarkGray  // Expired
    } else if elapsed_min < 20.0 {
        Color::Rgb(80, 220, 100)  // Green
    } else if elapsed_min < 40.0 {
        Color::Rgb(255, 200, 50)  // Yellow
    } else {
        Color::Rgb(255, 80, 80)  // Red
    };

    // 闪烁效果（最后5分钟）
    let in_flash_zone = elapsed_min >= 55.0 && elapsed < 3600;
    let flash_visible = if in_flash_zone {
        // 每500ms闪烁一次
        let phase = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() / 500) % 2;
        phase == 0
    } else {
        true
    };

    let mut spans = vec![
        Span::styled(" Cache ", Style::default().fg(Color::DarkGray)),
    ];

    if flash_visible {
        spans.push(Span::styled(
            hit_rate_text,
            Style::default().fg(hit_rate_color),
        ));
    }

    spans.push(Span::styled(" ", Style::default()));
    spans.push(Span::styled(
        countdown_text,
        Style::default().fg(timer_color),
    ));

    Line::from(spans)
}

/// 渲染简洁的缓存命中率（用于状态栏）
pub fn render_cache_pill_compact(stats: &CacheStatsState) -> String {
    let hit_rate = stats.hit_rate_percent()
        .map(|r| format!("{:.0}%", r))
        .unwrap_or_else(|| "--%".to_string());
    
    let countdown = stats.remaining_ttl_secs()
        .map(|s| format_countdown(s))
        .unwrap_or_else(|| "--:--".to_string());

    format!("Cache {} {}", hit_rate, countdown)
}
