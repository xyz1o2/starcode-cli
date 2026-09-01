/// 规则遮蔽检测
/// 
/// 对标claude-code-main的src/utils/permissions/shadowedRuleDetection.ts
/// 检测被其他规则遮蔽的权限规则

use serde::{Deserialize, Serialize};

/// 规则遮蔽检测器
pub struct ShadowedRuleDetector {
    /// 规则优先级
    priorities: HashMap<String, u32>,
}

/// 遮蔽检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowDetectionResult {
    /// 是否被遮蔽
    pub is_shadowed: bool,
    /// 遮蔽规则
    pub shadowed_by: Option<String>,
    /// 原因
    pub reason: String,
}

impl ShadowedRuleDetector {
    /// 创建新的规则遮蔽检测器
    pub fn new() -> Self {
        let mut priorities = HashMap::new();
        
        // 设置规则优先级
        priorities.insert("deny".to_string(), 100);
        priorities.insert("ask".to_string(), 50);
        priorities.insert("allow".to_string(), 10);
        
        Self { priorities }
    }

    /// 检测规则遮蔽
    pub fn detect_shadowing(
        &self,
        rules: &[PermissionRule],
        target_rule: &PermissionRule,
    ) -> ShadowDetectionResult {
        // 按优先级排序
        let mut sorted_rules: Vec<&PermissionRule> = rules.iter().collect();
        sorted_rules.sort_by(|a, b| {
            let a_priority = self.priorities.get(&a.action).unwrap_or(&0);
            let b_priority = self.priorities.get(&b.action).unwrap_or(&0);
            b_priority.cmp(a_priority)
        });

        // 检查是否有更高优先级的规则覆盖目标规则
        for rule in &sorted_rules {
            if rule.id == target_rule.id {
                continue;
            }

            if self.rule_covers(rule, target_rule) {
                return ShadowDetectionResult {
                    is_shadowed: true,
                    shadowed_by: Some(rule.id.clone()),
                    reason: format!(
                        "Rule '{}' ({}) is shadowed by rule '{}' ({})",
                        target_rule.id, target_rule.action,
                        rule.id, rule.action
                    ),
                };
            }
        }

        ShadowDetectionResult {
            is_shadowed: false,
            shadowed_by: None,
            reason: "Rule is not shadowed".to_string(),
        }
    }

    /// 检查规则是否覆盖目标规则
    fn rule_covers(&self, rule: &PermissionRule, target: &PermissionRule) -> bool {
        // 检查工具名称匹配
        if !self.tool_matches(&rule.tool_pattern, &target.tool_pattern) {
            return false;
        }

        // 检查命令模式匹配
        if let (Some(rule_cmd), Some(target_cmd)) = (&rule.command_pattern, &target.command_pattern) {
            if !self.command_matches(rule_cmd, target_cmd) {
                return false;
            }
        }

        // 检查优先级
        let rule_priority = self.priorities.get(&rule.action).unwrap_or(&0);
        let target_priority = self.priorities.get(&target.action).unwrap_or(&0);

        rule_priority > target_priority
    }

    /// 检查工具模式匹配
    fn tool_matches(&self, pattern: &str, target: &str) -> bool {
        if pattern == "*" || pattern == target {
            return true;
        }

        // 通配符匹配
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return target.starts_with(prefix);
        }

        false
    }

    /// 检查命令模式匹配
    fn command_matches(&self, pattern: &str, target: &str) -> bool {
        if pattern == "*" || pattern == target {
            return true;
        }

        // 前缀匹配
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return target.starts_with(prefix);
        }

        false
    }
}

/// 权限规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// 规则ID
    pub id: String,
    /// 工具模式
    pub tool_pattern: String,
    /// 命令模式
    pub command_pattern: Option<String>,
    /// 动作
    pub action: String,
    /// 来源
    pub source: String,
}
