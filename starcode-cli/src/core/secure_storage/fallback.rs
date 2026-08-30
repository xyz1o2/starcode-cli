/// 回退存储
/// 
/// 对标claude-code-main的fallbackStorage.ts

use super::types::{SecureStorage, StorageError};

/// 回退存储
/// 
/// 尝试多个存储后端，直到成功
pub struct FallbackStorage {
    /// 存储后端列表
    storages: Vec<Box<dyn SecureStorage>>,
}

impl FallbackStorage {
    /// 创建新的回退存储
    pub fn new() -> Self {
        Self {
            storages: Vec::new(),
        }
    }

    /// 添加存储后端
    pub fn add_storage(&mut self, storage: Box<dyn SecureStorage>) {
        self.storages.push(storage);
    }
}

impl SecureStorage for FallbackStorage {
    fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let mut last_error = StorageError::Unsupported;

        for storage in &self.storages {
            match storage.store(key, value) {
                Ok(()) => return Ok(()),
                Err(e) => last_error = e,
            }
        }

        Err(last_error)
    }

    fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let mut last_error = StorageError::Unsupported;

        for storage in &self.storages {
            match storage.get(key) {
                Ok(value) => return Ok(value),
                Err(e) => last_error = e,
            }
        }

        Err(last_error)
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        let mut last_error = StorageError::Unsupported;

        for storage in &self.storages {
            match storage.delete(key) {
                Ok(()) => return Ok(()),
                Err(e) => last_error = e,
            }
        }

        Err(last_error)
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        let mut last_error = StorageError::Unsupported;

        for storage in &self.storages {
            match storage.list() {
                Ok(list) => return Ok(list),
                Err(e) => last_error = e,
            }
        }

        Err(last_error)
    }

    fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let mut last_error = StorageError::Unsupported;

        for storage in &self.storages {
            match storage.exists(key) {
                Ok(exists) => return Ok(exists),
                Err(e) => last_error = e,
            }
        }

        Err(last_error)
    }
}
