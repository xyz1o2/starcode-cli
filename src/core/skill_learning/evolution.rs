/// 技能进化
/// 
/// 进化现有技能

use super::Skill;

/// 技能进化器
pub struct SkillEvolution {
    /// 进化阈值
    evolution_threshold: f64,
}

impl SkillEvolution {
    /// 创建新的技能进化器
    pub fn new() -> Self {
        Self {
            evolution_threshold: 0.8,
        }
    }

    /// 进化技能
    pub fn evolve(&self, skill: &Skill) -> Option<Skill> {
        // 检查是否需要进化
        if !self.should_evolve(skill) {
            return None;
        }

        let mut evolved = skill.clone();
        
        // 进化技能状态
        evolved.status = self.evolve_status(&skill.status);
        
        // 优化触发条件
        evolved.triggers = self.optimize_triggers(&skill.triggers);
        
        // 优化执行步骤
        evolved.steps = self.optimize_steps(&skill.steps);

        Some(evolved)
    }

    /// 检查是否应该进化
    fn should_evolve(&self, skill: &Skill) -> bool {
        // 使用次数足够多且成功率高
        skill.usage_count >= 5 && skill.success_rate >= self.evolution_threshold
    }

    /// 进化状态
    fn evolve_status(&self, current_status: &super::SkillStatus) -> super::SkillStatus {
        match current_status {
            super::SkillStatus::Observing => super::SkillStatus::Learning,
            super::SkillStatus::Learning => super::SkillStatus::Mastered,
            super::SkillStatus::Mastered => super::SkillStatus::Mastered,
            super::SkillStatus::Deprecated => super::SkillStatus::Deprecated,
        }
    }

    /// 优化触发条件
    fn optimize_triggers(&self, triggers: &[String]) -> Vec<String> {
        // 去重并排序
        let mut optimized: Vec<String> = triggers.to_vec();
        optimized.sort();
        optimized.dedup();
        optimized
    }

    /// 优化执行步骤
    fn optimize_steps(&self, steps: &[String]) -> Vec<String> {
        // 保持原样，未来可以添加优化逻辑
        steps.to_vec()
    }
}
