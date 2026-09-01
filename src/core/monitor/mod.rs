//! Monitor 后台监控模块
//!
//! 后台 shell 监控工具

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 监控配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub enabled: bool,
    pub check_interval_secs: u64,
    pub max_processes: usize,
    pub alert_threshold_cpu: f32,
    pub alert_threshold_memory_mb: u64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval_secs: 10,
            max_processes: 100,
            alert_threshold_cpu: 90.0,
            alert_threshold_memory_mb: 1024,
        }
    }
}

/// 监控进程
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredProcess {
    pub pid: u32,
    pub name: String,
    pub command: String,
    pub status: ProcessStatus,
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub started_at: u64,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessStatus {
    Running,
    Sleeping,
    Stopped,
    Zombie,
    Unknown,
}

/// 监控告警
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorAlert {
    pub alert_type: AlertType,
    pub message: String,
    pub severity: AlertSeverity,
    pub timestamp: u64,
    pub process: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertType {
    HighCpu,
    HighMemory,
    ProcessCrashed,
    DiskSpace,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// Monitor 管理器
pub struct MonitorManager {
    config: MonitorConfig,
    processes: Arc<Mutex<HashMap<u32, MonitoredProcess>>>,
    alerts: Arc<Mutex<Vec<MonitorAlert>>>,
}

impl MonitorManager {
    pub fn new(config: MonitorConfig) -> Self {
        Self {
            config,
            processes: Arc::new(Mutex::new(HashMap::new())),
            alerts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 启动监控
    pub async fn start(&self) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        let processes = self.processes.clone();
        let alerts = self.alerts.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            loop {
                Self::check_processes(&processes, &alerts, &config).await;
                tokio::time::sleep(std::time::Duration::from_secs(config.check_interval_secs))
                    .await;
            }
        });

        Ok(())
    }

    /// 检查进程
    async fn check_processes(
        processes: &Arc<Mutex<HashMap<u32, MonitoredProcess>>>,
        alerts: &Arc<Mutex<Vec<MonitorAlert>>>,
        config: &MonitorConfig,
    ) {
        // 读取 /proc 或使用 ps 命令
        let output = std::process::Command::new("ps")
            .args(["aux", "--no-headers"])
            .output();

        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut procs = processes.lock().unwrap();
            procs.clear();

            for line in stdout.lines().take(config.max_processes) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 11 {
                    let pid: u32 = parts[1].parse().unwrap_or(0);
                    let cpu: f32 = parts[2].parse().unwrap_or(0.0);
                    let mem_kb: u64 = parts[5].parse().unwrap_or(0);

                    procs.insert(
                        pid,
                        MonitoredProcess {
                            pid,
                            name: parts[10].to_string(),
                            command: parts[10..].join(" "),
                            status: ProcessStatus::Running,
                            cpu_percent: cpu,
                            memory_mb: mem_kb / 1024,
                            started_at: 0,
                            uptime_secs: 0,
                        },
                    );

                    // 检查告警
                    if cpu > config.alert_threshold_cpu {
                        alerts.lock().unwrap().push(MonitorAlert {
                            alert_type: AlertType::HighCpu,
                            message: format!(
                                "Process {} (PID {}) CPU usage: {:.1}%",
                                parts[10], pid, cpu
                            ),
                            severity: AlertSeverity::Warning,
                            timestamp: now_secs(),
                            process: Some(parts[10].to_string()),
                        });
                    }
                }
            }
        }
    }

    /// 获取所有监控进程
    pub fn list_processes(&self) -> Vec<MonitoredProcess> {
        self.processes.lock().unwrap().values().cloned().collect()
    }

    /// 获取告警
    pub fn get_alerts(&self) -> Vec<MonitorAlert> {
        self.alerts.lock().unwrap().clone()
    }

    /// 清除告警
    pub fn clear_alerts(&self) {
        self.alerts.lock().unwrap().clear();
    }

    /// 获取进程统计
    pub fn stats(&self) -> Value {
        let procs = self.processes.lock().unwrap();
        let alerts = self.alerts.lock().unwrap();

        json!({
            "process_count": procs.len(),
            "alert_count": alerts.len(),
            "total_cpu": procs.values().map(|p| p.cpu_percent as f64).sum::<f64>(),
            "total_memory_mb": procs.values().map(|p| p.memory_mb).sum::<u64>(),
        })
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
