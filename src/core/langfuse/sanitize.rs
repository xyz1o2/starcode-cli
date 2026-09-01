use serde_json::Value;

/// 数据清理器
/// 
/// 清理敏感信息，准备发送到Langfuse
pub struct DataSanitizer {
    /// 是否启用清理
    enabled: bool,
    /// 最大字段长度
    max_field_length: usize,
    /// 敏感字段模式
    sensitive_patterns: Vec<String>,
    /// 跳过的字段
    skip_fields: Vec<String>,
}

impl DataSanitizer {
    /// 创建新的数据清理器
    pub fn new() -> Self {
        Self {
            enabled: true,
            max_field_length: 10000,
            sensitive_patterns: vec![
                "api_key".to_string(),
                "secret".to_string(),
                "token".to_string(),
                "password".to_string(),
                "authorization".to_string(),
                "cookie".to_string(),
            ],
            skip_fields: Vec::new(),
        }
    }

    /// 设置是否启用清理
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 设置最大字段长度
    pub fn with_max_field_length(mut self, length: usize) -> Self {
        self.max_field_length = length;
        self
    }

    /// 添加敏感字段模式
    pub fn add_sensitive_pattern(&mut self, pattern: &str) {
        self.sensitive_patterns.push(pattern.to_string());
    }

    /// 添加跳过的字段
    pub fn add_skip_field(&mut self, field: &str) {
        self.skip_fields.push(field.to_string());
    }

    /// 清理JSON值
    pub fn sanitize_value(&self, value: Value) -> Value {
        if !self.enabled {
            return value;
        }

        match value {
            Value::Object(map) => {
                let mut sanitized = serde_json::Map::new();
                for (key, val) in map {
                    // 检查是否跳过该字段
                    if self.skip_fields.contains(&key) {
                        continue;
                    }

                    // 检查是否是敏感字段
                    if self.is_sensitive_field(&key) {
                        sanitized.insert(key, Value::String("[REDACTED]".to_string()));
                    } else {
                        sanitized.insert(key, self.sanitize_value(val));
                    }
                }
                Value::Object(sanitized)
            }
            Value::Array(arr) => {
                let sanitized: Vec<Value> = arr.into_iter()
                    .map(|v| self.sanitize_value(v))
                    .collect();
                Value::Array(sanitized)
            }
            Value::String(s) => {
                // 截断长字符串
                if s.len() > self.max_field_length {
                    let truncated = &s[..self.max_field_length];
                    Value::String(format!("{}...", truncated))
                } else {
                    Value::String(s)
                }
            }
            other => other,
        }
    }

    /// 检查是否是敏感字段
    fn is_sensitive_field(&self, field: &str) -> bool {
        let field_lower = field.to_lowercase();
        self.sensitive_patterns.iter().any(|pattern| {
            field_lower.contains(&pattern.to_lowercase())
        })
    }

    /// 清理消息内容
    pub fn sanitize_message_content(&self, content: &str) -> String {
        if !self.enabled {
            return content.to_string();
        }

        let mut result = content.to_string();

        // 简化的API密钥模式匹配（不使用正则表达式）
        let sensitive_patterns = [
            ("api_key=", "[REDACTED]"),
            ("api-key=", "[REDACTED]"),
            ("API_KEY=", "[REDACTED]"),
            ("secret=", "[REDACTED]"),
            ("SECRET=", "[REDACTED]"),
            ("token=", "[REDACTED]"),
            ("TOKEN=", "[REDACTED]"),
        ];

        for (pattern, replacement) in &sensitive_patterns {
            if result.contains(pattern) {
                // 简单的模式替换
                let parts: Vec<&str> = result.split(pattern).collect();
                if parts.len() > 1 {
                    let mut new_result = parts[0].to_string();
                    for i in 1..parts.len() {
                        // 替换等号后的值
                        let value_part = parts[i];
                        if let Some(end_pos) = value_part.find(|c: char| c.is_whitespace() || c == ',' || c == ';') {
                            new_result.push_str(pattern);
                            new_result.push_str(replacement);
                            new_result.push_str(&value_part[end_pos..]);
                        } else {
                            new_result.push_str(pattern);
                            new_result.push_str(replacement);
                        }
                    }
                    result = new_result;
                }
            }
        }

        // 移除文件路径中的用户目录
        if let Some(home_dir) = dirs::home_dir() {
            let home_str = home_dir.to_string_lossy().to_string();
            result = result.replace(&home_str, "~");
        }

        result
    }

    /// 清理工具参数
    pub fn sanitize_tool_arguments(&self, arguments: &str) -> String {
        if !self.enabled {
            return arguments.to_string();
        }

        // 尝试解析为JSON并清理
        if let Ok(value) = serde_json::from_str::<Value>(arguments) {
            let sanitized = self.sanitize_value(value);
            serde_json::to_string(&sanitized).unwrap_or_else(|_| arguments.to_string())
        } else {
            // 如果不是JSON，直接清理字符串
            self.sanitize_message_content(arguments)
        }
    }
}
