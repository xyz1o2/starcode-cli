/// 钥匙串存储
///
/// 对标claude-code-main的macOsKeychainStorage.ts
use super::types::{SecureStorage, StorageError};

/// 钥匙串存储
pub struct KeychainStorage {
    /// 服务名称
    service_name: String,
}

impl KeychainStorage {
    /// 创建新的钥匙串存储
    pub fn new() -> Self {
        Self {
            service_name: "starcode".to_string(),
        }
    }
}

impl SecureStorage for KeychainStorage {
    fn store(&self, key: &str, value: &str) -> Result<(), StorageError> {
        // TODO: 实现系统钥匙串存储
        // 在macOS上使用security命令或keychain-rs crate
        // 在Linux上使用secret-service
        // 在Windows上使用Windows Credential Manager

        #[cfg(target_os = "macos")]
        {
            // 使用security命令存储到macOS钥匙串
            let output = std::process::Command::new("security")
                .args([
                    "add-generic-password",
                    "-s",
                    &self.service_name,
                    "-a",
                    key,
                    "-w",
                    value,
                    "-U",
                ]) // 更新如果存在
                .output()
                .map_err(|e| StorageError::KeychainError(e.to_string()))?;

            if !output.status.success() {
                let error = String::from_utf8_lossy(&output.stderr);
                return Err(StorageError::KeychainError(error.to_string()));
            }

            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(StorageError::Unsupported)
        }
    }

    fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("security")
                .args([
                    "find-generic-password",
                    "-s",
                    &self.service_name,
                    "-a",
                    key,
                    "-w",
                ])
                .output()
                .map_err(|e| StorageError::KeychainError(e.to_string()))?;

            if output.status.success() {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(Some(value))
            } else {
                Ok(None)
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(StorageError::Unsupported)
        }
    }

    fn delete(&self, key: &str) -> Result<(), StorageError> {
        #[cfg(target_os = "macos")]
        {
            let output = std::process::Command::new("security")
                .args([
                    "delete-generic-password",
                    "-s",
                    &self.service_name,
                    "-a",
                    key,
                ])
                .output()
                .map_err(|e| StorageError::KeychainError(e.to_string()))?;

            if output.status.success() {
                Ok(())
            } else {
                Err(StorageError::NotFound)
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(StorageError::Unsupported)
        }
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        // 列出所有凭证比较复杂，这里返回空列表
        Ok(Vec::new())
    }

    fn exists(&self, key: &str) -> Result<bool, StorageError> {
        let result = self.get(key)?;
        Ok(result.is_some())
    }

    fn is_available(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            // 检查security命令是否可用
            std::process::Command::new("security")
                .arg("--version")
                .output()
                .is_ok()
        }

        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}
