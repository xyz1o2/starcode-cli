/// 目标追踪系统
///
/// 对标claude-code-main的src/services/goal/
/// 追踪用户的长期目标和任务，支持持久化和Agent集成
pub mod persistence;
pub mod prompts;

pub use persistence::GoalPersistence;
pub use prompts::GoalPrompts;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 目标状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GoalStatus {
    /// 进行中
    InProgress,
    /// 已完成
    Completed,
    /// 已暂停
    Paused,
    /// 已取消
    Cancelled,
}

/// 目标优先级
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum GoalPriority {
    /// 低
    Low,
    /// 中
    Medium,
    /// 高
    High,
    /// 紧急
    Critical,
}

/// 目标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    /// 目标ID
    pub id: String,
    /// 目标标题
    pub title: String,
    /// 目标描述
    pub description: Option<String>,
    /// 状态
    pub status: GoalStatus,
    /// 优先级
    pub priority: GoalPriority,
    /// 创建时间
    pub created_at: i64,
    /// 更新时间
    pub updated_at: i64,
    /// 完成时间
    pub completed_at: Option<i64>,
    /// 子目标
    pub sub_goals: Vec<String>,
    /// 标签
    pub tags: Vec<String>,
    /// 进度（0-100）
    pub progress: u8,
    /// 父目标ID
    pub parent_id: Option<String>,
    /// 截止时间
    pub deadline: Option<i64>,
    /// 里程碑
    pub milestones: Vec<Milestone>,
}

/// 里程碑
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    /// 里程碑ID
    pub id: String,
    /// 标题
    pub title: String,
    /// 是否完成
    pub completed: bool,
    /// 完成时间
    pub completed_at: Option<i64>,
}

/// 目标管理器
pub struct GoalManager {
    /// 目标存储
    goals: HashMap<String, Goal>,
    /// 持久化管理器
    persistence: GoalPersistence,
    /// 提示词管理器
    prompts: GoalPrompts,
}

impl GoalManager {
    pub fn new(storage_path: Option<&str>) -> Self {
        Self {
            goals: HashMap::new(),
            persistence: GoalPersistence::new(storage_path),
            prompts: GoalPrompts::new(),
        }
    }

    /// 从存储加载目标
    pub fn load(&mut self) -> Result<(), String> {
        let goals = self.persistence.load()?;
        self.goals = goals.into_iter().map(|g| (g.id.clone(), g)).collect();
        Ok(())
    }

    /// 保存目标到存储
    pub fn save(&self) -> Result<(), String> {
        let goals: Vec<&Goal> = self.goals.values().collect();
        self.persistence.save(&goals)
    }

    /// 创建新目标
    pub fn create_goal(
        &mut self,
        title: &str,
        description: Option<&str>,
        priority: GoalPriority,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        let goal = Goal {
            id: id.clone(),
            title: title.to_string(),
            description: description.map(|s| s.to_string()),
            status: GoalStatus::InProgress,
            priority,
            created_at: now,
            updated_at: now,
            completed_at: None,
            sub_goals: Vec::new(),
            tags: Vec::new(),
            progress: 0,
            parent_id: None,
            deadline: None,
            milestones: Vec::new(),
        };

        self.goals.insert(id.clone(), goal);
        let _ = self.save();
        id
    }

    /// 创建子目标
    pub fn create_sub_goal(
        &mut self,
        parent_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, String> {
        if !self.goals.contains_key(parent_id) {
            return Err(format!("Parent goal not found: {}", parent_id));
        }

        let id = self.create_goal(title, description, GoalPriority::Medium);

        if let Some(goal) = self.goals.get_mut(&id) {
            goal.parent_id = Some(parent_id.to_string());
        }

        if let Some(parent) = self.goals.get_mut(parent_id) {
            parent.sub_goals.push(id.clone());
        }

        let _ = self.save();
        Ok(id)
    }

    /// 更新目标状态
    pub fn update_status(&mut self, goal_id: &str, status: GoalStatus) -> Result<(), String> {
        if let Some(goal) = self.goals.get_mut(goal_id) {
            goal.status = status.clone();
            goal.updated_at = chrono::Utc::now().timestamp();

            if status == GoalStatus::Completed {
                goal.completed_at = Some(chrono::Utc::now().timestamp());
                goal.progress = 100;
            }

            let _ = self.save();
            Ok(())
        } else {
            Err(format!("Goal not found: {}", goal_id))
        }
    }

    /// 更新目标进度
    pub fn update_progress(&mut self, goal_id: &str, progress: u8) -> Result<(), String> {
        if let Some(goal) = self.goals.get_mut(goal_id) {
            goal.progress = progress.min(100);
            goal.updated_at = chrono::Utc::now().timestamp();

            if progress >= 100 {
                goal.status = GoalStatus::Completed;
                goal.completed_at = Some(chrono::Utc::now().timestamp());
            }

            let _ = self.save();
            Ok(())
        } else {
            Err(format!("Goal not found: {}", goal_id))
        }
    }

    /// 添加里程碑
    pub fn add_milestone(&mut self, goal_id: &str, title: &str) -> Result<String, String> {
        if let Some(goal) = self.goals.get_mut(goal_id) {
            let milestone_id = uuid::Uuid::new_v4().to_string();
            let milestone = Milestone {
                id: milestone_id.clone(),
                title: title.to_string(),
                completed: false,
                completed_at: None,
            };
            goal.milestones.push(milestone);
            goal.updated_at = chrono::Utc::now().timestamp();

            let _ = self.save();
            Ok(milestone_id)
        } else {
            Err(format!("Goal not found: {}", goal_id))
        }
    }

    /// 完成里程碑
    pub fn complete_milestone(&mut self, goal_id: &str, milestone_id: &str) -> Result<(), String> {
        if let Some(goal) = self.goals.get_mut(goal_id) {
            if let Some(milestone) = goal.milestones.iter_mut().find(|m| m.id == milestone_id) {
                milestone.completed = true;
                milestone.completed_at = Some(chrono::Utc::now().timestamp());
                goal.updated_at = chrono::Utc::now().timestamp();

                let _ = self.save();
                return Ok(());
            }
        }
        Err(format!("Goal or milestone not found"))
    }

    /// 获取目标
    pub fn get_goal(&self, goal_id: &str) -> Option<&Goal> {
        self.goals.get(goal_id)
    }

    /// 获取所有目标
    pub fn get_all_goals(&self) -> Vec<&Goal> {
        self.goals.values().collect()
    }

    /// 获取活跃目标
    pub fn get_active_goals(&self) -> Vec<&Goal> {
        self.goals
            .values()
            .filter(|g| g.status == GoalStatus::InProgress)
            .collect()
    }

    /// 获取子目标
    pub fn get_sub_goals(&self, parent_id: &str) -> Vec<&Goal> {
        self.goals
            .values()
            .filter(|g| g.parent_id.as_deref() == Some(parent_id))
            .collect()
    }

    /// 删除目标
    pub fn delete_goal(&mut self, goal_id: &str) -> Result<(), String> {
        if self.goals.remove(goal_id).is_some() {
            let _ = self.save();
            Ok(())
        } else {
            Err(format!("Goal not found: {}", goal_id))
        }
    }

    /// 生成目标提示词
    pub fn generate_prompt(&self, goal_id: &str) -> Result<String, String> {
        let goal = self
            .goals
            .get(goal_id)
            .ok_or_else(|| format!("Goal not found: {}", goal_id))?;

        Ok(self.prompts.generate_goal_prompt(goal))
    }
}
