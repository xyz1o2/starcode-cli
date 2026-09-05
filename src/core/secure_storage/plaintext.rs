/// 纯文本存储
///
/// 对标claude-code-main的plainTextStorage.ts
use super::types::{SecureStorage, StorageEntry, StorageError};
use std::collections::HashMap;
use std::path::PathBuf;

/// 纯文本存储
pub struct PlainTextStorage {
    /// 存储路径
    storage_path: PathBuf,
}

impl PlainTextStorage {
    /// 创建新的纯文本存储
    pub fn new(storage_path: &str) -> Self {
        let path = if storage_path.starts_with("~") {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(&storage_path[2..])
        } else {
            PathBuf::from(storage_path)
        };

        Self { storage_path: path }
    }

    /// 加载存储
    fn load_storage(&self) -> Result<HashMap<String, StorageEntry>, StorageError> {
        if !self.storage_path.exists() {
            return Ok(HashMap::new());
        }

        let content = std::fs::read_to_string(&self.storage_path)
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        serde_json::from_str(&content).map_err(|e| StorageError::SerializationError(e.to_string()))
    }

    /// 保存存储
    fn save_storage(&self, storage: &HashMap<String, StorageEntry>) -> Result<(), StorageError> {
        // 确保目录存在
        if let Some(parent) = self.storage_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StorageError::IoError(e.to_string()))?;
        }

        let content = serde_json::to_string_pretty(storage)
            .map_err(|e| StorageError::SerializationError(e.to_string()))?;

        std::fs::write(&self.storage_path, content)
            .map_err(|e| StorageError::IoError(e.to_string()))?;

        Ok(())
    }
}

impl SecureStorage for PlainTextStorage {
    fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let mut storage = self.load_storage()?;
        let now = chrono::Utc::now().timestamp();

        let entry = StorageEntry {
            key: key.to_string(),
            value: value.to_string(),
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
        };

        storage.insert(key.to_string(), entry);
        self.save_storage(&storage)
    }

    fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let storage = self.load_storage()?;
        Ok(storage.get(key).map(|entry| entry.value.clone()))
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        let mut storage = self.load_storage()?;
        storage.remove(key);
        self.save_storage(&storage)
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        let storage = self.load_storage()?;
        Ok(storage.keys().cloned().collect())
    }

    fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let storage = self.load_storage()?;
        Ok(storage.contains_key(key))
    }
}
