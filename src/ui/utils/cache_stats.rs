/// 缓存统计模块
/// 
/// 对标claude-code-main的src/utils/cacheStats.ts
/// 提供缓存统计功能

use std::collections::HashMap;

/// 缓存使用数据
#[derive(Debug, Clone)]
pub struct CacheUsage {
    /// 输入Token数量
    pub input_tokens: u32,
    /// 缓存创建Token数量
    pub cache_creation_input_tokens: u32,
    /// 缓存读取Token数量
    pub cache_read_input_tokens: u32,
}

/// 缓存统计状态
#[derive(Debug, Clone)]
pub struct CacheStatsState {
    /// 版本
    pub version: u32,
    /// 签名
    pub signature: Option<String>,
    /// 最后重置时间
    pub last_reset_at: Option<u64>,
    /// 最后命中率
    pub last_hit_rate: Option<f64>,
}

impl Default for CacheStatsState {
    fn default() -> Self {
        Self {
            version: 1,
            signature: None,
            last_reset_at: None,
            last_hit_rate: None,
        }
    }
}

/// 计算缓存命中率
/// 
/// 返回0-100的整数，如果分母为0则返回None
pub fn compute_hit_rate(usage: &CacheUsage) -> Option<f64> {
    let denom = usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens;
    if denom == 0 {
        return None;
    }
    Some((usage.cache_read_input_tokens as f64 / denom as f64) * 100.0)
}

/// 生成Token签名
/// 
/// 用于唯一标识一个使用快照
pub fn token_signature(usage: &CacheUsage) -> String {
    format!(
        "{}|{}|{}",
        usage.input_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens
    )
}

/// 缓存统计管理器
pub struct CacheStatsManager {
    /// 当前状态
    state: CacheStatsState,
    /// 历史数据
    history: Vec<CacheUsage>,
    /// 最大历史记录数
    max_history: usize,
}

impl CacheStatsManager {
    /// 创建新的缓存统计管理器
    pub fn new() -> Self {
        Self {
            state: CacheStatsState::default(),
            history: Vec::new(),
            max_history: 100,
        }
    }
    
    /// 更新缓存使用数据
    pub fn update(&mut self, usage: CacheUsage) {
        let signature = token_signature(&usage);
        let hit_rate = compute_hit_rate(&usage);
        
        // 如果签名变化，重置状态
        if self.state.signature.as_ref() != Some(&signature) {
            self.state.signature = Some(signature);
            self.state.last_reset_at = Some(chrono::Local::now().timestamp() as u64);
            self.state.last_hit_rate = hit_rate;
        }
        
        // 添加到历史记录
        self.history.push(usage);
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }
    
    /// 获取当前状态
    pub fn get_state(&self) -> &CacheStatsState {
        &self.state
    }
    
    /// 获取历史记录
    pub fn get_history(&self) -> &[CacheUsage] {
        &self.history
    }
    
    /// 获取平均命中率
    pub fn get_average_hit_rate(&self) -> Option<f64> {
        if self.history.is_empty() {
            return None;
        }
        
        let total_hit_rate: f64 = self.history
            .iter()
            .filter_map(|usage| compute_hit_rate(usage))
            .sum();
        
        let count = self.history
            .iter()
            .filter(|usage| compute_hit_rate(usage).is_some())
            .count();
        
        if count == 0 {
            return None;
        }
        
        Some(total_hit_rate / count as f64)
    }
    
    /// 获取最近N次的平均命中率
    pub fn get_recent_hit_rate(&self, n: usize) -> Option<f64> {
        let recent: Vec<&CacheUsage> = self.history
            .iter()
            .rev()
            .take(n)
            .collect();
        
        if recent.is_empty() {
            return None;
        }
        
        let total_hit_rate: f64 = recent
            .iter()
            .filter_map(|usage| compute_hit_rate(usage))
            .sum();
        
        let count = recent
            .iter()
            .filter(|usage| compute_hit_rate(usage).is_some())
            .count();
        
        if count == 0 {
            return None;
        }
        
        Some(total_hit_rate / count as f64)
    }
    
    /// 清空历史记录
    pub fn clear_history(&mut self) {
        self.history.clear();
    }
    
    /// 重置状态
    pub fn reset(&mut self) {
        self.state = CacheStatsState::default();
        self.history.clear();
    }
}

/// 创建缓存使用数据
pub fn create_cache_usage(
    input_tokens: u32,
    cache_creation_input_tokens: u32,
    cache_read_input_tokens: u32,
) -> CacheUsage {
    CacheUsage {
        input_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
    }
}

/// 格式化缓存命中率
pub fn format_hit_rate(hit_rate: f64) -> String {
    format!("{:.0}%", hit_rate)
}

/// 格式化缓存统计
pub fn format_cache_stats(manager: &CacheStatsManager) -> String {
    let state = manager.get_state();
    let hit_rate = state.last_hit_rate.unwrap_or(0.0);
    let hit_rate_str = format_hit_rate(hit_rate);
    
    let reset_time = state.last_reset_at
        .map(|t| {
            let now = chrono::Local::now().timestamp() as u64;
            let elapsed = now.saturating_sub(t);
            if elapsed >= 3600 {
                "exp".to_string()
            } else {
                let remaining = 3600 - elapsed;
                let minutes = remaining / 60;
                let seconds = remaining % 60;
                format!("{:02}:{:02}", minutes, seconds)
            }
        })
        .unwrap_or_else(|| "--:--".to_string());
    
    format!("Cache {} {}", hit_rate_str, reset_time)
}
