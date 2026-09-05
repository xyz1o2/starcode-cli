/// 技能学习系统
///
/// 对标claude-code-main的src/services/skillLearning/
/// 自动从用户行为中学习和进化技能
pub mod evolution;
pub mod generator;
pub mod instinct;
pub mod observer;
pub mod policy;
pub mod storage;

pub use evolution::SkillEvolution;
pub use generator::SkillGenerator;
pub use instinct::{InstinctParser, InstinctStore};
pub use observer::{Observation, SkillObserver};
pub use policy::LearningPolicy;
pub use storage::SkillStorage;

use serde::{Deserialize, Serialize};

/// 技能类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SkillType {
    /// Agent技能
    Agent,
    /// 命令技能
    Command,
    /// 工具技能
    Tool,
    /// 工作流技能
    Workflow,
    /// 本能技能
    Instinct,
}

/// 技能状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SkillStatus {
    /// 观察中
    Observing,
    /// 学习中
    Learning,
    /// 已掌握
    Mastered,
    /// 已弃用
    Deprecated,
}

/// 技能
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 技能ID
    pub id: String,
    /// 技能名称
    pub name: String,
    /// 技能类型
    pub skill_type: SkillType,
    /// 技能状态
    pub status: SkillStatus,
    /// 技能描述
    pub description: String,
    /// 触发条件
    pub triggers: Vec<String>,
    /// 执行步骤
    pub steps: Vec<String>,
    /// 使用次数
    pub usage_count: u32,
    /// 成功率
    pub success_rate: f64,
    /// 创建时间
    pub created_at: i64,
    /// 最后使用时间
    pub last_used_at: Option<i64>,
    /// 标签
    pub tags: Vec<String>,
}

/// 技能学习管理器
pub struct SkillLearningManager {
    /// 技能存储
    storage: SkillStorage,
    /// 技能观察器
    observer: SkillObserver,
    /// 技能生成器
    generator: SkillGenerator,
    /// 学习策略
    policy: LearningPolicy,
    /// 技能进化器
    evolution: SkillEvolution,
    /// 是否启用
    enabled: bool,
}

impl SkillLearningManager {
    /// 创建新的技能学习管理器
    pub fn new() -> Self {
        Self {
            storage: SkillStorage::new(),
            observer: SkillObserver::new(),
            generator: SkillGenerator::new(),
            policy: LearningPolicy::default(),
            evolution: SkillEvolution::new(),
            enabled: true,
        }
    }

    /// 从环境变量创建
    pub fn from_env() -> Self {
        let enabled = std::env::var("STAR_SKILL_LEARNING_ENABLED")
            .ok()
            .map(|v| v.to_lowercase() != "false" && v != "0")
            .unwrap_or(true);

        Self {
            storage: SkillStorage::new(),
            observer: SkillObserver::new(),
            generator: SkillGenerator::new(),
            policy: LearningPolicy::default(),
            evolution: SkillEvolution::new(),
            enabled,
        }
    }

    /// 观察用户行为
    pub fn observe(&mut self, observation: Observation) {
        if !self.enabled {
            return;
        }

        self.observer.record(observation);
    }

    /// 分析观察结果并生成技能
    pub fn analyze_and_generate(&mut self) -> Vec<Skill> {
        if !self.enabled {
            return Vec::new();
        }

        let observations = self.observer.get_pending_observations();
        let mut new_skills = Vec::new();

        for observation in observations {
            if let Some(skill) = self.generator.generate_from_observation(&observation) {
                new_skills.push(skill);
            }
        }

        // 存储新技能
        for skill in &new_skills {
            self.storage.add_skill(skill.clone());
        }

        new_skills
    }

    /// 进化现有技能
    pub fn evolve_skills(&mut self) {
        if !self.enabled {
            return;
        }

        // 先收集进化结果，避免与 get_all_skills 的不可变借用冲突
        let evolved: Vec<Skill> = self
            .storage
            .get_all_skills()
            .into_iter()
            .filter_map(|skill| self.evolution.evolve(skill))
            .collect();
        for skill in evolved {
            self.storage.update_skill(skill);
        }
    }

    /// 获取技能
    pub fn get_skill(&self, skill_id: &str) -> Option<&Skill> {
        self.storage.get_skill(skill_id)
    }

    /// 获取所有技能
    pub fn get_all_skills(&self) -> Vec<&Skill> {
        self.storage.get_all_skills()
    }

    /// 记录技能使用
    pub fn record_usage(&mut self, skill_id: &str, success: bool) {
        self.storage.record_usage(skill_id, success);
    }

    /// 检查是否启用
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
