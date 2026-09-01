use crate::llm::client::StarClient;
use crate::types::StarMessage;

/// 模型回退配置
#[derive(Debug, Clone)]
pub struct ModelFallbackConfig {
    /// 是否启用模型回退
    pub enabled: bool,
    /// 回退模型列表
    pub fallback_models: Vec<String>,
    /// 回退基础URL列表
    pub fallback_base_urls: Vec<String>,
    /// 最大重试次数
    pub max_retries: usize,
}

impl Default for ModelFallbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_models: Vec::new(),
            fallback_base_urls: Vec::new(),
            max_retries: 2,
        }
    }
}

impl ModelFallbackConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_MODEL_FALLBACK_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let mut fallback_models = Vec::new();
        if let Ok(model) = std::env::var("STAR_FALLBACK_MODEL") {
            fallback_models.push(model);
        }
        // 支持多个回退模型，用逗号分隔
        if let Ok(models) = std::env::var("STAR_FALLBACK_MODELS") {
            for model in models.split(',') {
                let trimmed = model.trim().to_string();
                if !trimmed.is_empty() && !fallback_models.contains(&trimmed) {
                    fallback_models.push(trimmed);
                }
            }
        }

        let mut fallback_base_urls = Vec::new();
        if let Ok(url) = std::env::var("STAR_FALLBACK_BASE_URL") {
            fallback_base_urls.push(url);
        }
        if let Ok(urls) = std::env::var("STAR_FALLBACK_BASE_URLS") {
            for url in urls.split(',') {
                let trimmed = url.trim().to_string();
                if !trimmed.is_empty() && !fallback_base_urls.contains(&trimmed) {
                    fallback_base_urls.push(trimmed);
                }
            }
        }

        let max_retries = std::env::var("STAR_MODEL_FALLBACK_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);

        Self {
            enabled,
            fallback_models,
            fallback_base_urls,
            max_retries,
        }
    }
}

/// 模型回退管理器
pub struct ModelFallbackManager {
    config: ModelFallbackConfig,
    /// 当前使用的模型索引
    current_model_index: usize,
    /// 当前使用的基础URL索引
    current_url_index: usize,
    /// 回退尝试次数
    retry_count: usize,
}

impl ModelFallbackManager {
    pub fn new() -> Self {
        let config = ModelFallbackConfig::from_env();
        Self {
            config,
            current_model_index: 0,
            current_url_index: 0,
            retry_count: 0,
        }
    }

    /// 检查是否是可回退的错误
    pub fn is_fallback_eligible_error(&self, error: &str) -> bool {
        let error_lower = error.to_lowercase();
        // 高负载、速率限制、服务不可用等错误可以回退
        error_lower.contains("overloaded")
            || error_lower.contains("rate_limit")
            || error_lower.contains("rate limit")
            || error_lower.contains("529")
            || error_lower.contains("503")
            || error_lower.contains("502")
            || error_lower.contains("service_unavailable")
            || error_lower.contains("too many requests")
    }

    /// 尝试回退到下一个模型
    pub fn try_fallback(&mut self, original_model: &str) -> FallbackDecision {
        if !self.config.enabled {
            return FallbackDecision::NoFallback {
                reason: "Fallback disabled".to_string(),
            };
        }

        if self.retry_count >= self.config.max_retries {
            return FallbackDecision::NoFallback {
                reason: format!(
                    "Max retries reached ({}/{})",
                    self.retry_count, self.config.max_retries
                ),
            };
        }

        // 尝试回退模型
        if self.current_model_index < self.config.fallback_models.len() {
            let fallback_model = &self.config.fallback_models[self.current_model_index];
            if fallback_model != original_model {
                self.current_model_index += 1;
                self.retry_count += 1;

                crate::utils::logging::append_debug_log_line(&format!(
                    "[MODEL_FALLBACK] Switching from {} to {} (attempt {}/{})",
                    original_model, fallback_model, self.retry_count, self.config.max_retries
                ));

                return FallbackDecision::Fallback {
                    model: fallback_model.clone(),
                    base_url: None,
                    reason: format!("Fallback to {}", fallback_model),
                };
            }
        }

        // 尝试回退基础URL
        if self.current_url_index < self.config.fallback_base_urls.len() {
            let fallback_url = &self.config.fallback_base_urls[self.current_url_index];
            self.current_url_index += 1;
            self.retry_count += 1;

            crate::utils::logging::append_debug_log_line(&format!(
                "[MODEL_FALLBACK] Switching base URL to {} (attempt {}/{})",
                fallback_url, self.retry_count, self.config.max_retries
            ));

            return FallbackDecision::Fallback {
                model: original_model.to_string(),
                base_url: Some(fallback_url.clone()),
                reason: format!("Fallback to URL {}", fallback_url),
            };
        }

        FallbackDecision::NoFallback {
            reason: "No more fallback options".to_string(),
        }
    }

    /// 重置回退状态
    pub fn reset(&mut self) {
        self.current_model_index = 0;
        self.current_url_index = 0;
        self.retry_count = 0;
    }

    /// 获取当前回退状态
    pub fn get_status(&self) -> FallbackStatus {
        FallbackStatus {
            enabled: self.config.enabled,
            retry_count: self.retry_count,
            max_retries: self.config.max_retries,
            available_fallbacks: self.config.fallback_models.len()
                + self.config.fallback_base_urls.len(),
        }
    }
}

/// 回退决策
#[derive(Debug, Clone)]
pub enum FallbackDecision {
    /// 执行回退
    Fallback {
        model: String,
        base_url: Option<String>,
        reason: String,
    },
    /// 不回退
    NoFallback { reason: String },
}

/// 回退状态
#[derive(Debug, Clone)]
pub struct FallbackStatus {
    pub enabled: bool,
    pub retry_count: usize,
    pub max_retries: usize,
    pub available_fallbacks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_fallback_eligible_error() {
        let manager = ModelFallbackManager::new();
        assert!(manager.is_fallback_eligible_error("Error: overloaded"));
        assert!(manager.is_fallback_eligible_error("rate_limit_exceeded"));
        assert!(manager.is_fallback_eligible_error("529 Too Many Requests"));
        assert!(!manager.is_fallback_eligible_error("invalid_api_key"));
    }
}
