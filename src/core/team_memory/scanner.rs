/// 密钥扫描器
/// 
/// 对标claude-code-main的secretScanner.ts
/// 检测内容中的敏感信息

/// 密钥扫描器
pub struct SecretScanner {
    /// 敏感模式
    patterns: Vec<String>,
}

impl SecretScanner {
    /// 创建新的密钥扫描器
    pub fn new() -> Self {
        let patterns = vec![
            // API密钥模式（简化版，避免正则表达式问题）
            "api_key=".to_string(),
            "api-key=".to_string(),
            "API_KEY=".to_string(),
            "secret=".to_string(),
            "SECRET=".to_string(),
            "token=".to_string(),
            "TOKEN=".to_string(),
            "password=".to_string(),
            "PASSWORD=".to_string(),
            // AWS密钥模式
            "AKIA".to_string(),
            // 私钥标记
            "-----BEGIN".to_string(),
            "PRIVATE KEY-----".to_string(),
        ];

        Self { patterns }
    }

    /// 检查内容是否包含密钥
    pub fn contains_secrets(&self, content: &str) -> bool {
        for pattern in &self.patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(content) {
                    return true;
                }
            }
        }
        false
    }

    /// 扫描并返回发现的密钥
    pub fn scan(&self, content: &str) -> Vec<SecretMatch> {
        let mut matches = Vec::new();
        let content_lower = content.to_lowercase();

        for pattern in &self.patterns {
            let pattern_lower = pattern.to_lowercase();
            if let Some(start) = content_lower.find(&pattern_lower) {
                let end = start + pattern.len();
                matches.push(SecretMatch {
                    pattern: pattern.clone(),
                    start,
                    end,
                    matched_text: content[start..end].to_string(),
                });
            }
        }

        matches
    }
}

/// 密钥匹配
#[derive(Debug)]
pub struct SecretMatch {
    /// 匹配的模式
    pub pattern: String,
    /// 开始位置
    pub start: usize,
    /// 结束位置
    pub end: usize,
    /// 匹配的文本
    pub matched_text: String,
}
