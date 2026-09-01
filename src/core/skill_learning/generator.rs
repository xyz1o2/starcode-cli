/// 技能生成器
/// 
/// 从观察结果生成技能

use super::{Skill, SkillType, SkillStatus};
use super::observer::Observation;

/// 技能生成器
pub struct SkillGenerator {
    /// 生成计数器
    generation_count: u32,
}

impl SkillGenerator {
    /// 创建新的技能生成器
    pub fn new() -> Self {
        Self {
            generation_count: 0,
        }
    }

    /// 从观察生成技能
    pub fn generate_from_observation(&mut self, observation: &Observation) -> Option<Skill> {
        // 检查是否可以生成技能
        if !self.can_generate_skill(observation) {
            return None;
        }

        self.generation_count += 1;

        let skill = Skill {
            id: uuid::Uuid::new_v4().to_string(),
            name: self.generate_skill_name(observation),
            skill_type: self.infer_skill_type(observation),
            status: SkillStatus::Observing,
            description: self.generate_description(observation),
            triggers: self.extract_triggers(observation),
            steps: self.extract_steps(observation),
            usage_count: 0,
            success_rate: 1.0,
            created_at: chrono::Utc::now().timestamp(),
            last_used_at: None,
            tags: observation.tags.clone(),
        };

        Some(skill)
    }

    /// 检查是否可以生成技能
    fn can_generate_skill(&self, observation: &Observation) -> bool {
        // 只有成功的观察才能生成技能
        if !observation.success {
            return false;
        }

        // 检查是否有足够的上下文
        if observation.context.is_empty() || observation.action.is_empty() {
            return false;
        }

        true
    }

    /// 生成技能名称
    fn generate_skill_name(&self, observation: &Observation) -> String {
        format!("skill_{}", self.generation_count)
    }

    /// 推断技能类型
    fn infer_skill_type(&self, observation: &Observation) -> SkillType {
        match observation.observation_type {
            super::observer::ObservationType::ToolCall => SkillType::Tool,
            super::observer::ObservationType::CommandExecution => SkillType::Command,
            super::observer::ObservationType::FileOperation => SkillType::Workflow,
            super::observer::ObservationType::ErrorFix => SkillType::Instinct,
            super::observer::ObservationType::Refactoring => SkillType::Workflow,
            super::observer::ObservationType::TestWriting => SkillType::Workflow,
        }
    }

    /// 生成描述
    fn generate_description(&self, observation: &Observation) -> String {
        format!("Learned from: {}", observation.context)
    }

    /// 提取触发条件
    fn extract_triggers(&self, observation: &Observation) -> Vec<String> {
        let mut triggers = Vec::new();
        
        // 从上下文中提取关键词作为触发条件
        let words: Vec<&str> = observation.context.split_whitespace().collect();
        for word in words.iter().take(5) {
            if word.len() > 3 {
                triggers.push(word.to_string());
            }
        }

        triggers
    }

    /// 提取执行步骤
    fn extract_steps(&self, observation: &Observation) -> Vec<String> {
        vec![observation.action.clone()]
    }
}
