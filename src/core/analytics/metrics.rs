use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 指标类型
#[derive(Debug, Clone)]
pub enum MetricType {
    /// 计数器（递增）
    Counter,
    /// 仪表盘（当前值）
    Gauge,
    /// 直方图（分布）
    Histogram,
    /// 计时器（持续时间）
    Timer,
}

/// 指标
#[derive(Debug, Clone)]
pub struct Metric {
    /// 指标名称
    pub name: String,
    /// 指标类型
    pub metric_type: MetricType,
    /// 指标值
    pub value: f64,
    /// 时间戳
    pub timestamp: u64,
    /// 标签
    pub tags: HashMap<String, String>,
}

impl Metric {
    /// 创建新指标
    pub fn new(name: &str, metric_type: MetricType, value: f64) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            name: name.to_string(),
            metric_type,
            value,
            timestamp,
            tags: HashMap::new(),
        }
    }

    /// 添加标签
    pub fn with_tag(mut self, key: &str, value: &str) -> Self {
        self.tags.insert(key.to_string(), value.to_string());
        self
    }
}

/// 指标收集器
///
/// 收集和管理系统指标
pub struct MetricsCollector {
    /// 指标存储
    metrics: HashMap<String, Vec<Metric>>,
    /// 计数器
    counters: HashMap<String, f64>,
    /// 仪表盘
    gauges: HashMap<String, f64>,
    /// 直方图
    histograms: HashMap<String, Vec<f64>>,
    /// 计时器
    timers: HashMap<String, Vec<f64>>,
    /// 是否启用
    enabled: bool,
}

impl MetricsCollector {
    /// 创建新的指标收集器
    pub fn new() -> Self {
        Self {
            metrics: HashMap::new(),
            counters: HashMap::new(),
            gauges: HashMap::new(),
            histograms: HashMap::new(),
            timers: HashMap::new(),
            enabled: true,
        }
    }

    /// 递增计数器
    pub fn increment_counter(&mut self, name: &str, value: f64) {
        if !self.enabled {
            return;
        }

        let counter = self.counters.entry(name.to_string()).or_insert(0.0);
        *counter += value;
        let new_value = *counter;

        self.record_metric(Metric::new(name, MetricType::Counter, new_value));
    }

    /// 设置仪表盘值
    pub fn set_gauge(&mut self, name: &str, value: f64) {
        if !self.enabled {
            return;
        }

        self.gauges.insert(name.to_string(), value);
        self.record_metric(Metric::new(name, MetricType::Gauge, value));
    }

    /// 记录直方图值
    pub fn record_histogram(&mut self, name: &str, value: f64) {
        if !self.enabled {
            return;
        }

        let histogram = self
            .histograms
            .entry(name.to_string())
            .or_insert_with(Vec::new);
        histogram.push(value);

        self.record_metric(Metric::new(name, MetricType::Histogram, value));
    }

    /// 记录计时器值（毫秒）
    pub fn record_timer(&mut self, name: &str, duration_ms: f64) {
        if !self.enabled {
            return;
        }

        let timer = self.timers.entry(name.to_string()).or_insert_with(Vec::new);
        timer.push(duration_ms);

        self.record_metric(Metric::new(name, MetricType::Timer, duration_ms));
    }

    /// 开始计时
    pub fn start_timer(&self, name: &str) -> TimerGuard {
        TimerGuard {
            name: name.to_string(),
            start: Instant::now(),
        }
    }

    /// 记录指标
    fn record_metric(&mut self, metric: Metric) {
        let metrics = self
            .metrics
            .entry(metric.name.clone())
            .or_insert_with(Vec::new);
        metrics.push(metric);
    }

    /// 获取计数器值
    pub fn get_counter(&self, name: &str) -> f64 {
        self.counters.get(name).copied().unwrap_or(0.0)
    }

    /// 获取仪表盘值
    pub fn get_gauge(&self, name: &str) -> Option<f64> {
        self.gauges.get(name).copied()
    }

    /// 获取直方图统计
    pub fn get_histogram_stats(&self, name: &str) -> Option<HistogramStats> {
        self.histograms.get(name).map(|values| {
            if values.is_empty() {
                return HistogramStats {
                    count: 0,
                    sum: 0.0,
                    min: 0.0,
                    max: 0.0,
                    mean: 0.0,
                    median: 0.0,
                    p95: 0.0,
                    p99: 0.0,
                };
            }

            let mut sorted = values.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

            let count = sorted.len();
            let sum: f64 = sorted.iter().sum();
            let min = sorted[0];
            let max = sorted[count - 1];
            let mean = sum / count as f64;
            let median = sorted[count / 2];
            let p95 = sorted[(count as f64 * 0.95) as usize];
            let p99 = sorted[(count as f64 * 0.99) as usize];

            HistogramStats {
                count,
                sum,
                min,
                max,
                mean,
                median,
                p95,
                p99,
            }
        })
    }

    /// 获取计时器统计
    pub fn get_timer_stats(&self, name: &str) -> Option<TimerStats> {
        self.get_histogram_stats(name).map(|hs| TimerStats {
            count: hs.count,
            total_ms: hs.sum,
            min_ms: hs.min,
            max_ms: hs.max,
            mean_ms: hs.mean,
            median_ms: hs.median,
            p95_ms: hs.p95,
            p99_ms: hs.p99,
        })
    }

    /// 获取所有指标名称
    pub fn metric_names(&self) -> Vec<String> {
        self.metrics.keys().cloned().collect()
    }

    /// 重置所有指标
    pub fn reset(&mut self) {
        self.metrics.clear();
        self.counters.clear();
        self.gauges.clear();
        self.histograms.clear();
        self.timers.clear();
    }

    /// 启用或禁用收集器
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// 计时器守卫
///
/// 自动记录计时器值
pub struct TimerGuard {
    name: String,
    start: Instant,
}

impl TimerGuard {
    /// 停止计时并返回持续时间（毫秒）
    pub fn stop(self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }
}

impl Drop for TimerGuard {
    fn drop(&mut self) {
        // 注意：这里不能直接调用MetricsCollector，因为所有权问题
        // 实际使用时需要手动调用stop()或在外部记录
    }
}

/// 直方图统计
#[derive(Debug, Clone)]
pub struct HistogramStats {
    pub count: usize,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
}

/// 计时器统计
#[derive(Debug, Clone)]
pub struct TimerStats {
    pub count: usize,
    pub total_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}
