/// 分析/遥测系统
///
/// 对标claude-code-main的src/services/analytics/
/// 提供使用分析、性能追踪和事件日志功能
pub mod config;
pub mod event_logger;
pub mod metrics;
pub mod sink;
pub mod telemetry;

pub use config::AnalyticsConfig;
pub use event_logger::{Event, EventLogger, EventType};
pub use metrics::{Metric, MetricsCollector};
pub use sink::{AnalyticsSink, SinkType};
pub use telemetry::TelemetryManager;
