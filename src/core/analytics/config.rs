/// 分析配置
/// 
/// 管理分析系统的配置选项
#[derive(Debug, Clone)]
pub struct AnalyticsConfig {
    /// 是否启用分析
    pub enabled: bool,
    /// 是否启用遥测
    pub telemetry_enabled: bool,
    /// 是否启用性能追踪
    pub performance_tracking: bool,
    /// 事件采样率（0.0-1.0）
    pub sampling_rate: f64,
    /// 最大事件缓冲区大小
    pub max_buffer_size: usize,
    /// 批量发送大小
    pub batch_size: usize,
    /// 发送间隔（秒）
    pub flush_interval_secs: u64,
    /// 是否启用用户识别
    pub user_identification: bool,
    /// 是否启用会话追踪
    pub session_tracking: bool,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            telemetry_enabled: true,
            performance_tracking: true,
            sampling_rate: 1.0,
            max_buffer_size: 1000,
            batch_size: 50,
            flush_interval_secs: 60,
            user_identification: false,
            session_tracking: true,
        }
    }
}

impl AnalyticsConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_ANALYTICS_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let telemetry_enabled = std::env::var("STAR_TELEMETRY_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let performance_tracking = std::env::var("STAR_PERFORMANCE_TRACKING")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let sampling_rate = std::env::var("STAR_ANALYTICS_SAMPLING_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);

        let max_buffer_size = std::env::var("STAR_ANALYTICS_BUFFER_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);

        let batch_size = std::env::var("STAR_ANALYTICS_BATCH_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);

        let flush_interval_secs = std::env::var("STAR_ANALYTICS_FLUSH_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let user_identification = std::env::var("STAR_ANALYTICS_USER_ID")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(false);

        let session_tracking = std::env::var("STAR_SESSION_TRACKING")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            enabled,
            telemetry_enabled,
            performance_tracking,
            sampling_rate,
            max_buffer_size,
            batch_size,
            flush_interval_secs,
            user_identification,
            session_tracking,
        }
    }

    /// 检查是否应该记录事件（基于采样率）
    pub fn should_sample(&self) -> bool {
        if !self.enabled {
            return false;
        }

        if self.sampling_rate >= 1.0 {
            return true;
        }

        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen::<f64>() < self.sampling_rate
    }

    /// 获取配置摘要
    pub fn summary(&self) -> String {
        format!(
            "AnalyticsConfig(enabled={}, telemetry={}, perf_tracking={}, sampling={:.1}%, buffer={}, batch={}, flush={}s)",
            self.enabled,
            self.telemetry_enabled,
            self.performance_tracking,
            self.sampling_rate * 100.0,
            self.max_buffer_size,
            self.batch_size,
            self.flush_interval_secs
        )
    }
}
