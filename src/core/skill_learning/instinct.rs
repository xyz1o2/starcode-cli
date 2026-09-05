/// 本能解析和存储
///
/// 对标claude-code-main的instinctParser.ts和instinctStore.ts
use serde::{Deserialize, Serialize};

/// 本能
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instinct {
    /// 本能ID
    pub id: String,
    /// 本能名称
    pub name: String,
    /// 触发条件
    pub trigger: String,
    /// 响应动作
    pub response: String,
    /// 优先级
    pub priority: u32,
    /// 使用次数
    pub usage_count: u32,
    /// 创建时间
    pub created_at: i64,
}

/// 本能解析器
pub struct InstinctParser {
    /// 本能模式
    patterns: Vec<InstinctPattern>,
}

/// 本能模式
#[derive(Debug, Clone)]
struct InstinctPattern {
    /// 模式名称
    name: String,
    /// 触发词
    triggers: Vec<String>,
    /// 响应模板
    response_template: String,
}

impl InstinctParser {
    /// 创建新的本能解析器
    pub fn new() -> Self {
        let mut parser = Self {
            patterns: Vec::new(),
        };

        parser.load_default_patterns();
        parser
    }

    /// 加载默认模式
    fn load_default_patterns(&mut self) {
        self.patterns.push(InstinctPattern {
            name: "error_fix".to_string(),
            triggers: vec!["error".to_string(), "fix".to_string(), "bug".to_string()],
            response_template: "Analyze the error and suggest a fix".to_string(),
        });

        self.patterns.push(InstinctPattern {
            name: "refactor".to_string(),
            triggers: vec![
                "refactor".to_string(),
                "improve".to_string(),
                "optimize".to_string(),
            ],
            response_template: "Suggest refactoring improvements".to_string(),
        });

        self.patterns.push(InstinctPattern {
            name: "test".to_string(),
            triggers: vec![
                "test".to_string(),
                "verify".to_string(),
                "check".to_string(),
            ],
            response_template: "Write or run tests".to_string(),
        });
    }

    /// 解析输入
    pub fn parse(&self, input: &str) -> Option<Instinct> {
        let input_lower = input.to_lowercase();

        for pattern in &self.patterns {
            for trigger in &pattern.triggers {
                if input_lower.contains(trigger) {
                    return Some(Instinct {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: pattern.name.clone(),
                        trigger: trigger.clone(),
                        response: pattern.response_template.clone(),
                        priority: 1,
                        usage_count: 0,
                        created_at: chrono::Utc::now().timestamp(),
                    });
                }
            }
        }

        None
    }
}

/// 本能存储
pub struct InstinctStore {
    /// 本能映射
    instincts: std::collections::HashMap<String, Instinct>,
}

impl InstinctStore {
    /// 创建新的本能存储
    pub fn new() -> Self {
        Self {
            instincts: std::collections::HashMap::new(),
        }
    }

    /// 添加本能
    pub fn add_instinct(&mut self, instinct: Instinct) {
        self.instincts.insert(instinct.id.clone(), instinct);
    }

    /// 获取本能
    pub fn get_instinct(&self, instinct_id: &str) -> Option<&Instinct> {
        self.instincts.get(instinct_id)
    }

    /// 获取所有本能
    pub fn get_all_instincts(&self) -> Vec<&Instinct> {
        self.instincts.values().collect()
    }

    /// 删除本能
    pub fn delete_instinct(&mut self, instinct_id: &str) {
        self.instincts.remove(instinct_id);
    }

    /// 记录使用
    pub fn record_usage(&mut self, instinct_id: &str) {
        if let Some(instinct) = self.instincts.get_mut(instinct_id) {
            instinct.usage_count += 1;
        }
    }
}
