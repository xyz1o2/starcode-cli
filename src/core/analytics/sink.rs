use super::event_logger::Event;
use super::metrics::Metric;

/// Sink类型
#[derive(Debug, Clone)]
pub enum SinkType {
    /// 控制台输出
    Console,
    /// 文件输出
    File(String),
    /// HTTP端点
    Http(String),
    /// 数据库
    Database(String),
    /// 自定义
    Custom(String),
}

/// 分析Sink
///
/// 将事件和指标发送到不同的目标
pub struct AnalyticsSink {
    /// Sink类型
    sink_type: SinkType,
    /// 是否启用
    enabled: bool,
    /// 缓冲区
    buffer: Vec<SinkEntry>,
    /// 最大缓冲区大小
    max_buffer_size: usize,
}

/// Sink条目
#[derive(Debug, Clone)]
pub enum SinkEntry {
    /// 事件
    Event(Event),
    /// 指标
    Metric(Metric),
}

impl AnalyticsSink {
    /// 创建新的分析Sink
    pub fn new(sink_type: SinkType, max_buffer_size: usize) -> Self {
        Self {
            sink_type,
            enabled: true,
            buffer: Vec::with_capacity(max_buffer_size),
            max_buffer_size,
        }
    }

    /// 发送事件
    pub fn send_event(&mut self, event: Event) {
        if !self.enabled {
            return;
        }

        if self.buffer.len() >= self.max_buffer_size {
            self.flush();
        }

        self.buffer.push(SinkEntry::Event(event));
    }

    /// 发送指标
    pub fn send_metric(&mut self, metric: Metric) {
        if !self.enabled {
            return;
        }

        if self.buffer.len() >= self.max_buffer_size {
            self.flush();
        }

        self.buffer.push(SinkEntry::Metric(metric));
    }

    /// 刷新缓冲区
    pub fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let entries: Vec<SinkEntry> = self.buffer.drain(..).collect();

        match &self.sink_type {
            SinkType::Console => {
                for entry in &entries {
                    match entry {
                        SinkEntry::Event(event) => {
                            println!("[Analytics Event] {:?}", event);
                        }
                        SinkEntry::Metric(metric) => {
                            println!("[Analytics Metric] {:?}", metric);
                        }
                    }
                }
            }
            SinkType::File(path) => {
                // 文件输出实现
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap();

                use std::io::Write;
                for entry in &entries {
                    match entry {
                        SinkEntry::Event(event) => {
                            writeln!(file, "[Event] {:?}", event).unwrap();
                        }
                        SinkEntry::Metric(metric) => {
                            writeln!(file, "[Metric] {:?}", metric).unwrap();
                        }
                    }
                }
            }
            SinkType::Http(url) => {
                // HTTP输出实现（简化版）
                println!(
                    "[Analytics HTTP] Would send {} entries to {}",
                    entries.len(),
                    url
                );
            }
            SinkType::Database(conn) => {
                // 数据库输出实现（简化版）
                println!(
                    "[Analytics DB] Would store {} entries to {}",
                    entries.len(),
                    conn
                );
            }
            SinkType::Custom(name) => {
                // 自定义输出实现（简化版）
                println!(
                    "[Analytics Custom] Would process {} entries via {}",
                    entries.len(),
                    name
                );
            }
        }
    }

    /// 启用或禁用Sink
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// 获取缓冲区大小
    pub fn buffer_size(&self) -> usize {
        self.buffer.len()
    }

    /// 获取Sink类型
    pub fn sink_type(&self) -> &SinkType {
        &self.sink_type
    }
}

/// Sink管理器
///
/// 管理多个Sink
pub struct SinkManager {
    sinks: Vec<AnalyticsSink>,
}

impl SinkManager {
    /// 创建新的Sink管理器
    pub fn new() -> Self {
        Self { sinks: Vec::new() }
    }

    /// 添加Sink
    pub fn add_sink(&mut self, sink: AnalyticsSink) {
        self.sinks.push(sink);
    }

    /// 发送事件到所有Sink
    pub fn broadcast_event(&mut self, event: Event) {
        for sink in &mut self.sinks {
            sink.send_event(event.clone());
        }
    }

    /// 发送指标到所有Sink
    pub fn broadcast_metric(&mut self, metric: Metric) {
        for sink in &mut self.sinks {
            sink.send_metric(metric.clone());
        }
    }

    /// 刷新所有Sink
    pub fn flush_all(&mut self) {
        for sink in &mut self.sinks {
            sink.flush();
        }
    }

    /// 获取Sink数量
    pub fn sink_count(&self) -> usize {
        self.sinks.len()
    }
}
