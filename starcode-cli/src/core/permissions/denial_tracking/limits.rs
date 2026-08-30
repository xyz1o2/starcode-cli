/// 拒绝限制配置

/// 拒绝限制
#[derive(Debug, Clone)]
pub struct DenialLimits {
    /// 最大连续拒绝次数
    pub max_consecutive_denials: u32,
    /// 最大总拒绝次数
    pub max_total_denials: u32,
    /// 拒绝冷却时间（秒）
    pub cooldown_secs: u64,
}

impl Default for DenialLimits {
    fn default() -> Self {
        Self {
            max_consecutive_denials: 5,
            max_total_denials: 20,
            cooldown_secs: 60,
        }
    }
}

impl DenialLimits {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let max_consecutive_denials = std::env::var("STAR_DENIAL_MAX_CONSECUTIVE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);

        let max_total_denials = std::env::var("STAR_DENIAL_MAX_TOTAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let cooldown_secs = std::env::var("STAR_DENIAL_COOLDOWN_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        Self {
            max_consecutive_denials,
            max_total_denials,
            cooldown_secs,
        }
    }
}
