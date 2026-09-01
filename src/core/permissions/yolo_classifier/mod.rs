/// YOLO分类器
/// 
/// 对标claude-code-main的src/utils/permissions/yoloClassifier.ts
/// LLM驱动的命令安全分类

pub mod classifier;
pub mod prompts;
pub mod types;

pub use classifier::YoloClassifier;
pub use prompts::ClassifierPrompts;
pub use types::{ClassifierResult, ClassifierBehavior, ClassifierInput};

use serde::{Deserialize, Serialize};

/// 分类器配置
#[derive(Debug, Clone)]
pub struct ClassifierConfig {
    /// 是否启用
    pub enabled: bool,
    /// 使用的模型
    pub model: String,
    /// 超时（秒）
    pub timeout_secs: u64,
    /// 最大重试次数
    pub max_retries: u32,
    /// 置信度阈值
    pub confidence_threshold: f64,
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "gpt-4o-mini".to_string(),
            timeout_secs: 10,
            max_retries: 2,
            confidence_threshold: 0.8,
        }
    }
}

impl ClassifierConfig {
    /// 从环境变量加载配置
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_YOLO_CLASSIFIER_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        let model = std::env::var("STAR_YOLO_CLASSIFIER_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());

        let timeout_secs = std::env::var("STAR_YOLO_CLASSIFIER_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let max_retries = std::env::var("STAR_YOLO_CLASSIFIER_MAX_RETRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);

        let confidence_threshold = std::env::var("STAR_YOLO_CLASSIFIER_CONFIDENCE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.8);

        Self {
            enabled,
            model,
            timeout_secs,
            max_retries,
            confidence_threshold,
        }
    }
}

/// YOLO分类器管理器
pub struct YoloClassifierManager {
    config: ClassifierConfig,
    classifier: YoloClassifier,
}

impl YoloClassifierManager {
    /// 创建新的YOLO分类器管理器
    pub fn new(config: ClassifierConfig) -> Self {
        let classifier = YoloClassifier::new(config.clone());
        Self { config, classifier }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        Self::new(ClassifierConfig::from_env())
    }

    /// 分类命令
    pub async fn classify(&self, input: &ClassifierInput) -> ClassifierResult {
        if !self.config.enabled {
            return ClassifierResult {
                matches: false,
                confidence: "high".to_string(),
                reason: "Classifier disabled".to_string(),
                behavior: ClassifierBehavior::Allow,
            };
        }

        self.classifier.classify(input).await
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}
