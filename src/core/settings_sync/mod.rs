/// 设置同步系统
///
/// 对标claude-code-main的src/services/settingsSync/
/// 跨设备设置同步
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 同步配置
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// 是否启用
    pub enabled: bool,
    /// 同步端点
    pub endpoint: Option<String>,
    /// 认证令牌
    pub auth_token: Option<String>,
    /// 同步间隔（秒）
    pub sync_interval_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            auth_token: None,
            sync_interval_secs: 300,
        }
    }
}

/// 同步状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncStatus {
    /// 未同步
    NotSynced,
    /// 同步中
    Syncing,
    /// 已同步
    Synced,
    /// 同步失败
    Failed(String),
}

/// 设置条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingEntry {
    /// 设置键
    pub key: String,
    /// 设置值
    pub value: serde_json::Value,
    /// 最后修改时间
    pub last_modified: i64,
    /// 设备ID
    pub device_id: String,
}

/// 设置同步管理器
pub struct SettingsSyncManager {
    config: SyncConfig,
    settings: HashMap<String, SettingEntry>,
    sync_status: SyncStatus,
    device_id: String,
}

impl SettingsSyncManager {
    pub fn new(config: SyncConfig) -> Self {
        let device_id = uuid::Uuid::new_v4().to_string();

        Self {
            config,
            settings: HashMap::new(),
            sync_status: SyncStatus::NotSynced,
            device_id,
        }
    }

    /// 获取设置
    pub fn get_setting(&self, key: &str) -> Option<&SettingEntry> {
        self.settings.get(key)
    }

    /// 设置值
    pub fn set_setting(&mut self, key: &str, value: serde_json::Value) {
        let entry = SettingEntry {
            key: key.to_string(),
            value,
            last_modified: chrono::Utc::now().timestamp(),
            device_id: self.device_id.clone(),
        };
        self.settings.insert(key.to_string(), entry);
    }

    /// 删除设置
    pub fn delete_setting(&mut self, key: &str) {
        self.settings.remove(key);
    }

    /// 获取所有设置
    pub fn get_all_settings(&self) -> &HashMap<String, SettingEntry> {
        &self.settings
    }

    /// 导出设置为JSON
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.settings)
    }

    /// 从JSON导入设置
    pub fn import_json(&mut self, json: &str) -> Result<(), serde_json::Error> {
        let imported: HashMap<String, SettingEntry> = serde_json::from_str(json)?;
        self.settings.extend(imported);
        Ok(())
    }

    /// 获取同步状态
    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync_status
    }

    /// 获取设备ID
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}
