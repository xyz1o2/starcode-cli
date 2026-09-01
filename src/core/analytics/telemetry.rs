use super::config::AnalyticsConfig;
use super::event_logger::{Event, EventLogger, EventType};
use super::metrics::{MetricsCollector, TimerGuard};
use super::sink::{AnalyticsSink, SinkManager, SinkType};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// 遥测管理器
///
/// 对标claude-code-main的遥测系统
/// 提供统一的遥测数据收集和发送接口
pub struct TelemetryManager {
    /// 配置
    config: AnalyticsConfig,
    /// 事件日志器
    event_logger: EventLogger,
    /// 指标收集器
    metrics_collector: MetricsCollector,
    /// Sink管理器
    sink_manager: SinkManager,
    /// 会话ID
    session_id: Option<String>,
    /// 用户ID
    user_id: Option<String>,
    /// 启动时间
    start_time: SystemTime,
}

impl TelemetryManager {
    /// 创建新的遥测管理器
    pub fn new(config: AnalyticsConfig) -> Self {
        let event_logger = EventLogger::new(config.max_buffer_size);
        let metrics_collector = MetricsCollector::new();
        let sink_manager = SinkManager::new();

        Self {
            config,
            event_logger,
            metrics_collector,
            sink_manager,
            session_id: None,
            user_id: None,
            start_time: SystemTime::now(),
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(AnalyticsConfig::from_env())
    }

    /// 设置会话ID
    pub fn set_session_id(&mut self, session_id: &str) {
        self.session_id = Some(session_id.to_string());
    }

    /// 设置用户ID
    pub fn set_user_id(&mut self, user_id: &str) {
        self.user_id = Some(user_id.to_string());
    }

    /// 添加Sink
    pub fn add_sink(&mut self, sink_type: SinkType) {
        let sink = AnalyticsSink::new(sink_type, self.config.max_buffer_size);
        self.sink_manager.add_sink(sink);
    }

    /// 记录事件
    pub fn log_event(&mut self, event_type: EventType) {
        if !self.config.should_sample() {
            return;
        }

        let mut event = Event::new(event_type);

        if let Some(session_id) = &self.session_id {
            event = event.with_session_id(session_id);
        }

        if let Some(user_id) = &self.user_id {
            event = event.with_user_id(user_id);
        }

        self.event_logger.log(event.clone());
        self.sink_manager.broadcast_event(event);
    }

    /// 记录带数据的事件
    pub fn log_event_with_data(
        &mut self,
        event_type: EventType,
        data: HashMap<String, serde_json::Value>,
    ) {
        if !self.config.should_sample() {
            return;
        }

        let mut event = Event::new(event_type);

        if let Some(session_id) = &self.session_id {
            event = event.with_session_id(session_id);
        }

        if let Some(user_id) = &self.user_id {
            event = event.with_user_id(user_id);
        }

        for (key, value) in data {
            event = event.with_data(&key, value);
        }

        self.event_logger.log(event.clone());
        self.sink_manager.broadcast_event(event);
    }

    /// 记录工具调用
    pub fn log_tool_call(&mut self, tool_name: &str, arguments: &str) {
        let mut data = HashMap::new();
        data.insert(
            "tool_name".to_string(),
            serde_json::Value::String(tool_name.to_string()),
        );
        data.insert(
            "arguments".to_string(),
            serde_json::Value::String(arguments.to_string()),
        );

        self.log_event_with_data(EventType::ToolCall, data);
    }

    /// 记录工具结果
    pub fn log_tool_result(&mut self, tool_name: &str, success: bool, duration_ms: u64) {
        let mut data = HashMap::new();
        data.insert(
            "tool_name".to_string(),
            serde_json::Value::String(tool_name.to_string()),
        );
        data.insert("success".to_string(), serde_json::Value::Bool(success));
        data.insert(
            "duration_ms".to_string(),
            serde_json::Value::Number(duration_ms.into()),
        );

        self.log_event_with_data(EventType::ToolResult, data);

        // 记录性能指标
        self.metrics_collector
            .record_timer(&format!("tool.{}.duration", tool_name), duration_ms as f64);
        if !success {
            self.metrics_collector
                .increment_counter(&format!("tool.{}.errors", tool_name), 1.0);
        }
    }

    /// 记录模型响应
    pub fn log_model_response(&mut self, model: &str, tokens_used: u32, duration_ms: u64) {
        let mut data = HashMap::new();
        data.insert(
            "model".to_string(),
            serde_json::Value::String(model.to_string()),
        );
        data.insert(
            "tokens_used".to_string(),
            serde_json::Value::Number(tokens_used.into()),
        );
        data.insert(
            "duration_ms".to_string(),
            serde_json::Value::Number(duration_ms.into()),
        );

        self.log_event_with_data(EventType::ModelResponse, data);

        // 记录性能指标
        self.metrics_collector
            .record_timer("model.response_time", duration_ms as f64);
        self.metrics_collector
            .record_histogram("model.tokens_used", tokens_used as f64);
    }

    /// 记录错误
    pub fn log_error(&mut self, error_type: &str, message: &str) {
        let mut data = HashMap::new();
        data.insert(
            "error_type".to_string(),
            serde_json::Value::String(error_type.to_string()),
        );
        data.insert(
            "message".to_string(),
            serde_json::Value::String(message.to_string()),
        );

        self.log_event_with_data(EventType::Error, data);

        // 记录错误计数
        self.metrics_collector
            .increment_counter("errors.total", 1.0);
        self.metrics_collector
            .increment_counter(&format!("errors.{}", error_type), 1.0);
    }

    /// 记录压缩事件
    pub fn log_compaction(&mut self, tokens_before: usize, tokens_after: usize, strategy: &str) {
        let mut data = HashMap::new();
        data.insert(
            "tokens_before".to_string(),
            serde_json::Value::Number(tokens_before.into()),
        );
        data.insert(
            "tokens_after".to_string(),
            serde_json::Value::Number(tokens_after.into()),
        );
        data.insert(
            "strategy".to_string(),
            serde_json::Value::String(strategy.to_string()),
        );
        data.insert(
            "tokens_saved".to_string(),
            serde_json::Value::Number((tokens_before - tokens_after).into()),
        );

        self.log_event_with_data(EventType::Compaction, data);

        // 记录压缩指标
        self.metrics_collector.record_histogram(
            "compaction.tokens_saved",
            (tokens_before - tokens_after) as f64,
        );
        self.metrics_collector
            .increment_counter("compaction.count", 1.0);
    }

    /// 记录会话开始
    pub fn log_session_start(&mut self, session_id: &str) {
        self.set_session_id(session_id);

        let mut data = HashMap::new();
        data.insert(
            "session_id".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );

        self.log_event_with_data(EventType::Session, data);

        // 记录会话计数
        self.metrics_collector
            .increment_counter("sessions.total", 1.0);
    }

    /// 记录会话结束
    pub fn log_session_end(&mut self, session_id: &str, duration_ms: u64) {
        let mut data = HashMap::new();
        data.insert(
            "session_id".to_string(),
            serde_json::Value::String(session_id.to_string()),
        );
        data.insert(
            "duration_ms".to_string(),
            serde_json::Value::Number(duration_ms.into()),
        );

        self.log_event_with_data(EventType::Session, data);

        // 记录会话时长
        self.metrics_collector
            .record_timer("sessions.duration", duration_ms as f64);
    }

    /// 开始计时
    pub fn start_timer(&self, name: &str) -> TimerGuard {
        self.metrics_collector.start_timer(name)
    }

    /// 记录指标
    pub fn record_metric(&mut self, name: &str, value: f64) {
        self.metrics_collector.set_gauge(name, value);
    }

    /// 递增计数器
    pub fn increment_counter(&mut self, name: &str, value: f64) {
        self.metrics_collector.increment_counter(name, value);
    }

    /// 刷新所有数据
    pub fn flush(&mut self) {
        self.sink_manager.flush_all();
    }

    /// 获取事件日志器引用
    pub fn event_logger(&self) -> &EventLogger {
        &self.event_logger
    }

    /// 获取指标收集器引用
    pub fn metrics_collector(&self) -> &MetricsCollector {
        &self.metrics_collector
    }

    /// 获取运行时间（毫秒）
    pub fn uptime_ms(&self) -> u64 {
        self.start_time.elapsed().unwrap_or_default().as_millis() as u64
    }

    /// 获取摘要统计
    pub fn summary(&self) -> TelemetrySummary {
        TelemetrySummary {
            session_id: self.session_id.clone(),
            user_id: self.user_id.clone(),
            uptime_ms: self.uptime_ms(),
            total_events: self.event_logger.total_count(),
            total_metrics: self.metrics_collector.metric_names().len(),
            sink_count: self.sink_manager.sink_count(),
        }
    }
}

/// 遥测摘要
#[derive(Debug)]
pub struct TelemetrySummary {
    pub session_id: Option<String>,
    pub user_id: Option<String>,
    pub uptime_ms: u64,
    pub total_events: u64,
    pub total_metrics: usize,
    pub sink_count: usize,
}
