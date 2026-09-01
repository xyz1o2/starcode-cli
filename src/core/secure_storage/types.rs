/// 安全存储类型定义

use serde::{Deserialize, Serialize};

/// 存储条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEntry {
    /// 键
    pub key: String,
    /// 值
    pub value: String,
    /// 创建时间
    pub created_at: i64,
    /// 更新时间
    pub updated_at: i64,
    /// 元数据
    pub metadata: std::collections::HashMap<String, String>,
}

/// 存储错误
#[derive(Debug)]
pub enum StorageError {
    /// 未找到
    NotFound,
    /// 权限错误
    PermissionDenied,
    /// 加密错误
    EncryptionError(String),
    /// 解密错误
    DecryptionError(String),
    /// IO错误
    IoError(String),
    /// 序列化错误
    SerializationError(String),
    /// 钥匙串错误
    KeychainError(String),
    /// 不支持的操作
    Unsupported,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::NotFound => write!(f, "Storage entry not found"),
            StorageError::PermissionDenied => write!(f, "Permission denied"),
            StorageError::EncryptionError(e) => write!(f, "Encryption error: {}", e),
            StorageError::DecryptionError(e) => write!(f, "Decryption error: {}", e),
            StorageError::IoError(e) => write!(f, "IO error: {}", e),
            StorageError::SerializationError(e) => write!(f, "Serialization error: {}", e),
            StorageError::KeychainError(e) => write!(f, "Keychain error: {}", e),
            StorageError::Unsupported => write!(f, "Unsupported operation"),
        }
    }
}

impl std::error::Error for StorageError {}

/// 安全存储trait
pub trait SecureStorage: Send + Sync {
    /// 存储凭证
    fn store(&self, key: &str, value: &str) -> Result<(), StorageError>;
    
    /// 获取凭证
    fn get(&self, key: &str) -> Result<Option<String>, StorageError>;
    
    /// 删除凭证
    fn delete(&self, key: &str) -> Result<(), StorageError>;
    
    /// 列出所有凭证
    fn list(&self) -> Result<Vec<String>, StorageError>;
    
    /// 检查凭证是否存在
    fn exists(&self, key: &str) -> Result<bool, StorageError>;
    
    /// 检查存储是否可用
    fn is_available(&self) -> bool {
        true
    }
}
