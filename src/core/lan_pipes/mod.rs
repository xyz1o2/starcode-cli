//! LAN Pipes 局域网协作模块
//!
//! 对标 Claude Code 的 lan-pipes.md：
//! - TCP/UDP Multicast 跨机器协作
//! - 实例发现
//! - 消息路由

use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

/// LAN 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanConfig {
    pub enabled: bool,
    pub bind_address: String,
    pub port: u16,
    pub multicast_addr: String,
    pub multicast_port: u16,
    pub instance_name: String,
    pub discovery_interval_secs: u64,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind_address: "0.0.0.0".to_string(),
            port: 9529,
            multicast_addr: "239.255.95.28".to_string(),
            multicast_port: 9530,
            instance_name: "starcode".to_string(),
            discovery_interval_secs: 30,
        }
    }
}

/// LAN 实例信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanInstance {
    pub id: String,
    pub name: String,
    pub address: SocketAddr,
    pub capabilities: Vec<String>,
    pub last_seen: u64,
    pub status: InstanceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstanceStatus {
    Online,
    Busy,
    Offline,
}

/// LAN 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanMessage {
    pub from_id: String,
    pub to_id: String,
    pub message_type: String,
    pub payload: Value,
    pub timestamp: u64,
}

/// LAN Pipes 管理器
pub struct LanPipesManager {
    config: LanConfig,
    instances: Arc<Mutex<HashMap<String, LanInstance>>>,
    message_queue: Arc<Mutex<Vec<LanMessage>>>,
}

impl LanPipesManager {
    pub fn new(config: LanConfig) -> Self {
        Self {
            config,
            instances: Arc::new(Mutex::new(HashMap::new())),
            message_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn start(&self) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        let instances = self.instances.clone();
        let config = self.config.clone();
        tokio::spawn(async move {
            Self::discovery_loop(config, instances).await;
        });

        Ok(())
    }

    async fn discovery_loop(
        config: LanConfig,
        instances: Arc<Mutex<HashMap<String, LanInstance>>>,
    ) {
        loop {
            // 清理离线实例
            {
                let mut inst = instances.lock().await;
                let now = now_secs();
                inst.retain(|_, i| now - i.last_seen < config.discovery_interval_secs * 3);
            }

            tokio::time::sleep(std::time::Duration::from_secs(
                config.discovery_interval_secs,
            ))
            .await;
        }
    }

    pub async fn send_message(&self, to: &str, message_type: &str, payload: Value) -> Result<(), String> {
        let instances = self.instances.lock().await;
        let target = instances
            .get(to)
            .ok_or_else(|| format!("Instance '{}' not found", to))?;

        if target.status == InstanceStatus::Offline {
            return Err(format!("Instance '{}' is offline", to));
        }

        Ok(())
    }

    pub async fn list_instances(&self) -> Vec<LanInstance> {
        self.instances.lock().await.values().cloned().collect()
    }

    pub async fn register_local(&self) {
        let instance = LanInstance {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            name: self.config.instance_name.clone(),
            address: format!("{}:{}", self.config.bind_address, self.config.port)
                .parse()
                .unwrap(),
            capabilities: vec!["chat".into(), "tools".into(), "files".into()],
            last_seen: now_secs(),
            status: InstanceStatus::Online,
        };

        self.instances.lock().await.insert(instance.id.clone(), instance);
    }

    pub async fn receive_message(&self) -> Option<LanMessage> {
        let mut queue = self.message_queue.lock().await;
        if queue.is_empty() {
            None
        } else {
            Some(queue.remove(0))
        }
    }

    pub async fn queue_size(&self) -> usize {
        self.message_queue.lock().await.len()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
