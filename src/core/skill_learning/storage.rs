/// 技能存储
///
/// 持久化存储技能
use super::Skill;
use std::collections::HashMap;

/// 技能存储
pub struct SkillStorage {
    /// 技能映射
    skills: HashMap<String, Skill>,
}

impl SkillStorage {
    /// 创建新的技能存储
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
        }
    }

    /// 添加技能
    pub fn add_skill(&mut self, skill: Skill) {
        self.skills.insert(skill.id.clone(), skill);
    }

    /// 获取技能
    pub fn get_skill(&self, skill_id: &str) -> Option<&Skill> {
        self.skills.get(skill_id)
    }

    /// 获取所有技能
    pub fn get_all_skills(&self) -> Vec<&Skill> {
        self.skills.values().collect()
    }

    /// 更新技能
    pub fn update_skill(&mut self, skill: Skill) {
        self.skills.insert(skill.id.clone(), skill);
    }

    /// 删除技能
    pub fn delete_skill(&mut self, skill_id: &str) {
        self.skills.remove(skill_id);
    }

    /// 记录使用
    pub fn record_usage(&mut self, skill_id: &str, success: bool) {
        if let Some(skill) = self.skills.get_mut(skill_id) {
            skill.usage_count += 1;
            skill.last_used_at = Some(chrono::Utc::now().timestamp());

            // 更新成功率
            let total = skill.usage_count as f64;
            let current_successes = skill.success_rate * (total - 1.0);
            let new_successes = if success {
                current_successes + 1.0
            } else {
                current_successes
            };
            skill.success_rate = new_successes / total;
        }
    }

    /// 清理过期技能
    pub fn cleanup_expired(&mut self, max_age_secs: i64) {
        let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
        self.skills.retain(|_, skill| {
            skill
                .last_used_at
                .map_or(true, |last_used| last_used > cutoff)
        });
    }
}
