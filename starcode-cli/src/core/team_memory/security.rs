/// 安全守卫
/// 
/// 对标claude-code-main的teamMemSecretGuard.ts
/// 确保团队记忆不包含敏感信息

use super::TeamMemoryError;

/// 安全守卫
pub struct SecurityGuard {
    /// 禁止的模式
    forbidden_patterns: Vec<String>,
    /// 最大内容长度
    max_content_length: usize,
}

impl SecurityGuard {
    /// 创建新的安全守卫
    pub fn new() -> Self {
        let forbidden_patterns = vec![
            // 可能包含密钥的模式
            "private key".to_string(),
            "secret key".to_string(),
            "api key".to_string(),
            "password".to_string(),
            "credential".to_string(),
        ];

        Self {
            forbidden_patterns,
            max_content_length: 10000, // 10KB
        }
    }

    /// 检查内容
    pub fn check_content(&self, content: &str) -> Result<(), TeamMemoryError> {
        // 检查长度
        if content.len() > self.max_content_length {
            return Err(TeamMemoryError::SecurityCheckFailed(
                format!("Content too large: {} bytes (max: {})", content.len(), self.max_content_length)
            ));
        }

        // 检查禁止的模式
        let content_lower = content.to_lowercase();
        for pattern in &self.forbidden_patterns {
            if content_lower.contains(pattern) {
                return Err(TeamMemoryError::SecurityCheckFailed(
                    format!("Content contains forbidden pattern: {}", pattern)
                ));
            }
        }

        Ok(())
    }

    /// 添加禁止的模式
    pub fn add_forbidden_pattern(&mut self, pattern: String) {
        self.forbidden_patterns.push(pattern);
    }
}
