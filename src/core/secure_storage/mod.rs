/// 安全存储系统
///
/// 对标claude-code-main的src/utils/secureStorage/
/// 提供安全的凭证存储功能
pub mod fallback;
pub mod keychain;
pub mod plaintext;
pub mod types;

pub use fallback::FallbackStorage;
pub use keychain::KeychainStorage;
pub use plaintext::PlainTextStorage;
pub use types::{SecureStorage, StorageEntry, StorageError};

use serde::{Deserialize, Serialize};

/// 存储后端类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StorageBackend {
    /// 系统钥匙串
    Keychain,
    /// 纯文本文件
    PlainText,
    /// 自动选择
    Auto,
}

/// 安全存储配置
#[derive(Debug, Clone)]
pub struct SecureStorageConfig {
    /// 存储后端
    pub backend: StorageBackend,
    /// 存储路径（用于PlainText后端）
    pub storage_path: Option<String>,
    /// 是否启用加密
    pub encryption_enabled: bool,
    /// 加密密钥
    pub encryption_key: Option<String>,
}

impl Default for SecureStorageConfig {
    fn default() -> Self {
        Self {
            backend: StorageBackend::Auto,
            storage_path: None,
            encryption_enabled: false,
            encryption_key: None,
        }
    }
}

impl SecureStorageConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let backend = std::env::var("STAR_SECURE_STORAGE_BACKEND")
            .ok()
            .map(|v| match v.to_lowercase().as_str() {
                "keychain" => StorageBackend::Keychain,
                "plaintext" => StorageBackend::PlainText,
                _ => StorageBackend::Auto,
            })
            .unwrap_or(StorageBackend::Auto);

        let storage_path = std::env::var("STAR_SECURE_STORAGE_PATH").ok();

        let encryption_enabled = std::env::var("STAR_SECURE_STORAGE_ENCRYPTION")
            .ok()
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        let encryption_key = std::env::var("STAR_SECURE_STORAGE_KEY").ok();

        Self {
            backend,
            storage_path,
            encryption_enabled,
            encryption_key,
        }
    }
}

/// 安全存储管理器
pub struct SecureStorageManager {
    /// 配置
    config: SecureStorageConfig,
    /// 存储后端
    storage: Box<dyn SecureStorage>,
}

impl SecureStorageManager {
    /// 创建新的安全存储管理器
    pub fn new(config: SecureStorageConfig) -> Self {
        let storage: Box<dyn SecureStorage> = match config.backend {
            StorageBackend::Keychain => Box::new(KeychainStorage::new()),
            StorageBackend::PlainText => {
                let path = config
                    .storage_path
                    .clone()
                    .unwrap_or_else(|| "~/.star/secure_storage.json".to_string());
                Box::new(PlainTextStorage::new(&path))
            }
            StorageBackend::Auto => {
                // 尝试使用钥匙串，失败则回退到纯文本
                let keychain = KeychainStorage::new();
                if keychain.is_available() {
                    Box::new(keychain)
                } else {
                    let path = config
                        .storage_path
                        .clone()
                        .unwrap_or_else(|| "~/.star/secure_storage.json".to_string());
                    Box::new(PlainTextStorage::new(&path))
                }
            }
        };

        Self { config, storage }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(SecureStorageConfig::from_env())
    }

    /// 存储凭证
    pub fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.storage.store(key, value)
    }

    /// 获取凭证
    pub fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.storage.get(key)
    }

    /// 删除凭证
    pub fn delete(&self, key: &str) -> Result<(), StorageError> {
        self.storage.delete(key)
    }

    /// 列出所有凭证
    pub fn list(&self) -> Result<Vec<String>, StorageError> {
        self.storage.list()
    }

    /// 检查凭证是否存在
    pub fn exists(&self, key: &str) -> Result<bool, StorageError> {
        self.storage.exists(key)
    }

    /// 获取存储后端类型
    pub fn backend_type(&self) -> StorageBackend {
        self.config.backend.clone()
    }
}
