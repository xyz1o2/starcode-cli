/// 意图归一化器
///
/// 将用户输入规范化为标准意图
use serde::{Deserialize, Serialize};

/// 归一化的意图
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedIntent {
    /// 意图类型
    pub intent_type: IntentType,
    /// 置信度
    pub confidence: f64,
    /// 提取的关键词
    pub keywords: Vec<String>,
    /// 原始输入
    pub original_input: String,
}

/// 意图类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum IntentType {
    /// 代码编辑
    CodeEdit,
    /// 代码搜索
    CodeSearch,
    /// 代码解释
    CodeExplanation,
    /// 测试运行
    TestRun,
    /// Git操作
    GitOperation,
    /// 文件操作
    FileOperation,
    /// 配置更改
    ConfigChange,
    /// 问题诊断
    ProblemDiagnosis,
    /// 其他
    Other,
}

/// 意图归一化器
pub struct IntentNormalizer {
    /// 关键词映射
    keyword_map: std::collections::HashMap<String, IntentType>,
}

impl IntentNormalizer {
    pub fn new() -> Self {
        let mut keyword_map = std::collections::HashMap::new();

        // 代码编辑关键词
        for word in &[
            "edit", "modify", "change", "update", "fix", "refactor", "rewrite",
        ] {
            keyword_map.insert(word.to_string(), IntentType::CodeEdit);
        }

        // 搜索关键词
        for word in &["find", "search", "locate", "where", "grep"] {
            keyword_map.insert(word.to_string(), IntentType::CodeSearch);
        }

        // 解释关键词
        for word in &["explain", "describe", "what", "how", "why", "understand"] {
            keyword_map.insert(word.to_string(), IntentType::CodeExplanation);
        }

        // 测试关键词
        for word in &["test", "run", "check", "verify", "validate"] {
            keyword_map.insert(word.to_string(), IntentType::TestRun);
        }

        // Git关键词
        for word in &["git", "commit", "push", "pull", "branch", "merge"] {
            keyword_map.insert(word.to_string(), IntentType::GitOperation);
        }

        // 文件操作关键词
        for word in &["create", "delete", "move", "copy", "rename"] {
            keyword_map.insert(word.to_string(), IntentType::FileOperation);
        }

        // 配置关键词
        for word in &["config", "setting", "env", "environment"] {
            keyword_map.insert(word.to_string(), IntentType::ConfigChange);
        }

        // 诊断关键词
        for word in &["error", "bug", "issue", "problem", "debug", "troubleshoot"] {
            keyword_map.insert(word.to_string(), IntentType::ProblemDiagnosis);
        }

        Self { keyword_map }
    }

    /// 归一化用户输入
    pub fn normalize(&self, input: &str) -> NormalizedIntent {
        let input_lower = input.to_lowercase();
        let words: Vec<&str> = input_lower.split_whitespace().collect();

        let mut intent_scores: std::collections::HashMap<IntentType, f64> =
            std::collections::HashMap::new();
        let mut keywords = Vec::new();

        for word in &words {
            if let Some(intent_type) = self.keyword_map.get(*word) {
                *intent_scores.entry(intent_type.clone()).or_insert(0.0) += 1.0;
                keywords.push(word.to_string());
            }
        }

        // 找到最高分的意图
        let (intent_type, score) = intent_scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(k, v)| (k.clone(), *v))
            .unwrap_or((IntentType::Other, 0.0));

        let confidence = if words.is_empty() {
            0.0
        } else {
            score / words.len() as f64
        };

        NormalizedIntent {
            intent_type,
            confidence,
            keywords,
            original_input: input.to_string(),
        }
    }
}
