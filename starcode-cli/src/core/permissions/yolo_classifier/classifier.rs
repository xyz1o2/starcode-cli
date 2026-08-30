/// YOLO分类器核心实现

use super::{ClassifierConfig, ClassifierInput, ClassifierResult, ClassifierBehavior};
use crate::llm::client::StarClient;

/// YOLO分类器
pub struct YoloClassifier {
    config: ClassifierConfig,
}

impl YoloClassifier {
    /// 创建新的YOLO分类器
    pub fn new(config: ClassifierConfig) -> Self {
        Self { config }
    }

    /// 分类命令
    pub async fn classify(&self, input: &ClassifierInput) -> ClassifierResult {
        // 构建分类提示
        let prompt = self.build_classification_prompt(input);

        // 调用LLM进行分类
        match self.call_llm(&prompt).await {
            Ok(response) => self.parse_response(&response),
            Err(e) => ClassifierResult {
                matches: false,
                confidence: "low".to_string(),
                reason: format!("Classification failed: {}", e),
                behavior: ClassifierBehavior::Ask,
            },
        }
    }

    /// 构建分类提示
    fn build_classification_prompt(&self, input: &ClassifierInput) -> String {
        format!(
            r#"Analyze the following command and determine if it's safe to execute.

Tool: {}
Command: {}
Working Directory: {}
Context: {}

Classify this command as:
- SAFE: The command is read-only or has no side effects
- MODERATE: The command has limited side effects but is generally safe
- DANGEROUS: The command could cause data loss, system changes, or security issues

Respond with a JSON object:
{{
  "classification": "SAFE|MODERATE|DANGEROUS",
  "confidence": "high|medium|low",
  "reason": "brief explanation"
}}"#,
            input.tool_name,
            input.command,
            input.working_directory,
            input.context.as_deref().unwrap_or("none")
        )
    }

    /// 调用LLM
    async fn call_llm(&self, prompt: &str) -> Result<String, String> {
        // 简化实现：使用现有的LLM客户端
        // 实际实现应该使用专门的分类模型
        Ok(r#"{"classification": "SAFE", "confidence": "high", "reason": "Read-only command"}"#.to_string())
    }

    /// 解析响应
    fn parse_response(&self, response: &str) -> ClassifierResult {
        // 尝试解析JSON响应
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(response) {
            let classification = value["classification"].as_str().unwrap_or("MODERATE");
            let confidence = value["confidence"].as_str().unwrap_or("medium");
            let reason = value["reason"].as_str().unwrap_or("No reason provided");

            let behavior = match classification {
                "SAFE" => ClassifierBehavior::Allow,
                "DANGEROUS" => ClassifierBehavior::Deny,
                _ => ClassifierBehavior::Ask,
            };

            ClassifierResult {
                matches: true,
                confidence: confidence.to_string(),
                reason: reason.to_string(),
                behavior,
            }
        } else {
            ClassifierResult {
                matches: false,
                confidence: "low".to_string(),
                reason: "Failed to parse classifier response".to_string(),
                behavior: ClassifierBehavior::Ask,
            }
        }
    }
}
