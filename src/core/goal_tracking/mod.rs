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

    /// 先保存候选状态，成功后才替换内存；失败不会残留半次更新。
    fn commit<T>(
        &mut self,
        mutate: impl FnOnce(&mut HashMap<String, Goal>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut goals = self.goals.clone();
        let result = mutate(&mut goals)?;
        self.persistence.save(&goals.values().collect::<Vec<_>>())?;
        self.goals = goals;
        Ok(result)
    }

    /// 创建尚未持久化的目标，供普通创建与子目标事务共用。
    fn new_goal(title: &str, description: Option<&str>, priority: GoalPriority) -> Goal {
        let now = chrono::Utc::now().timestamp();
        Goal {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            description: description.map(str::to_string),
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
        }
    }

    /// 创建新目标
    pub fn create_goal(
        &mut self,
        title: &str,
        description: Option<&str>,
        priority: GoalPriority,
    ) -> Result<String, String> {
        let goal = Self::new_goal(title, description, priority);
        self.commit(|goals| {
            let id = goal.id.clone();
            goals.insert(id.clone(), goal);
            Ok(id)
        })
    }

    /// 创建子目标
    pub fn create_sub_goal(
        &mut self,
        parent_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, String> {
        self.commit(|goals| {
            let parent = goals
                .get_mut(parent_id)
                .ok_or_else(|| format!("Parent goal not found: {}", parent_id))?;
            let mut goal = Self::new_goal(title, description, GoalPriority::Medium);
            goal.parent_id = Some(parent_id.to_string());
            let id = goal.id.clone();
            parent.sub_goals.push(id.clone());
            parent.updated_at = goal.created_at;
            goals.insert(id.clone(), goal);
            Ok(id)
        })
    }

    /// 更新目标状态
    pub fn update_status(&mut self, goal_id: &str, status: GoalStatus) -> Result<(), String> {
        self.commit(|goals| {
            let goal = goals
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal not found: {}", goal_id))?;
            let was_completed = goal.status == GoalStatus::Completed;
            goal.status = status;
            goal.updated_at = chrono::Utc::now().timestamp();
            if goal.status == GoalStatus::Completed {
                goal.completed_at = Some(goal.updated_at);
                goal.progress = 100;
            } else {
                goal.completed_at = None;
                if was_completed {
                    goal.progress = 0;
                }
            }
            Ok(())
        })
    }

    /// 更新目标进度
    pub fn update_progress(&mut self, goal_id: &str, progress: u8) -> Result<(), String> {
        if progress > 100 {
            return Err(crate::core::i18n::t1("cmd.goal.invalid_progress")
                .replace("{value}", &progress.to_string()));
        }
        self.commit(|goals| {
            let goal = goals
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal not found: {}", goal_id))?;
            goal.progress = progress;
            goal.updated_at = chrono::Utc::now().timestamp();
            if progress == 100 {
                goal.status = GoalStatus::Completed;
                goal.completed_at = Some(goal.updated_at);
            } else {
                if goal.status == GoalStatus::Completed {
                    goal.status = GoalStatus::InProgress;
                }
                goal.completed_at = None;
            }
            Ok(())
        })
    }

    /// 添加里程碑
    pub fn add_milestone(&mut self, goal_id: &str, title: &str) -> Result<String, String> {
        self.commit(|goals| {
            let goal = goals
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal not found: {}", goal_id))?;
            let milestone_id = uuid::Uuid::new_v4().to_string();
            goal.milestones.push(Milestone {
                id: milestone_id.clone(),
                title: title.to_string(),
                completed: false,
                completed_at: None,
            });
            goal.updated_at = chrono::Utc::now().timestamp();
            Ok(milestone_id)
        })
    }

    /// 完成里程碑
    pub fn complete_milestone(&mut self, goal_id: &str, milestone_id: &str) -> Result<(), String> {
        self.commit(|goals| {
            let goal = goals
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal not found: {}", goal_id))?;
            let milestone = goal
                .milestones
                .iter_mut()
                .find(|m| m.id == milestone_id)
                .ok_or_else(|| format!("Milestone not found: {}", milestone_id))?;
            let now = chrono::Utc::now().timestamp();
            milestone.completed = true;
            milestone.completed_at = Some(now);
            goal.updated_at = now;
            Ok(())
        })
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
        self.commit(|goals| {
            goals
                .remove(goal_id)
                .ok_or_else(|| format!("Goal not found: {}", goal_id))?;
            Ok(())
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(dir: &std::path::Path) -> GoalManager {
        GoalManager::new(Some(dir.join("goals.json").to_str().unwrap()))
    }

    #[test]
    fn lowering_completed_progress_reopens_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let id = mgr.create_goal("Test", None, GoalPriority::Medium).unwrap();
        mgr.update_status(&id, GoalStatus::Completed).unwrap();
        assert!(mgr.get_goal(&id).unwrap().completed_at.is_some());
        mgr.update_progress(&id, 50).unwrap();
        let mut reloaded = manager(dir.path());
        reloaded.load().unwrap();
        let goal = reloaded.get_goal(&id).unwrap();
        assert_eq!(goal.status, GoalStatus::InProgress);
        assert_eq!(goal.progress, 50);
        assert_eq!(goal.completed_at, None);
        assert_eq!(reloaded.get_active_goals().len(), 1);
    }

    #[test]
    fn progress_preserves_paused_and_cancelled_status_below_completion() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let id = mgr.create_goal("Test", None, GoalPriority::Medium).unwrap();
        for status in [GoalStatus::Paused, GoalStatus::Cancelled] {
            mgr.update_status(&id, status.clone()).unwrap();
            mgr.update_progress(&id, 40).unwrap();
            assert_eq!(mgr.get_goal(&id).unwrap().status, status);
            assert_eq!(mgr.get_goal(&id).unwrap().completed_at, None);
        }
        mgr.update_progress(&id, 100).unwrap();
        assert_eq!(mgr.get_goal(&id).unwrap().status, GoalStatus::Completed);
    }

    #[test]
    fn leaving_completed_clears_completion_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let id = mgr.create_goal("Test", None, GoalPriority::Medium).unwrap();
        for status in [
            GoalStatus::InProgress,
            GoalStatus::Paused,
            GoalStatus::Cancelled,
        ] {
            mgr.update_status(&id, GoalStatus::Completed).unwrap();
            mgr.update_status(&id, status.clone()).unwrap();
            assert_eq!(mgr.get_goal(&id).unwrap().status, status);
            assert_eq!(mgr.get_goal(&id).unwrap().completed_at, None);
            assert_eq!(mgr.get_goal(&id).unwrap().progress, 0);
        }
    }

    #[test]
    fn out_of_range_progress_is_rejected_without_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let id = mgr.create_goal("Test", None, GoalPriority::Medium).unwrap();
        let before = std::fs::read(dir.path().join("goals.json")).unwrap();
        for progress in [101, 255] {
            assert!(mgr.update_progress(&id, progress).is_err());
            assert_eq!(mgr.get_goal(&id).unwrap().progress, 0);
            assert_eq!(mgr.get_goal(&id).unwrap().status, GoalStatus::InProgress);
        }
        assert_eq!(
            std::fs::read(dir.path().join("goals.json")).unwrap(),
            before
        );
    }

    #[test]
    fn every_failed_write_preserves_memory_and_existing_storage() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let id = mgr.create_goal("Test", None, GoalPriority::Medium).unwrap();
        let milestone = mgr.add_milestone(&id, "Step").unwrap();
        let before = serde_json::to_value(&mgr.goals).unwrap();
        let stored = std::fs::read(dir.path().join("goals.json")).unwrap();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "block").unwrap();
        mgr.persistence = GoalPersistence::new(Some(blocker.join("goals.json").to_str().unwrap()));
        assert!(mgr.create_goal("New", None, GoalPriority::Medium).is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert!(mgr.create_sub_goal(&id, "Child", None).is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert!(mgr.update_status(&id, GoalStatus::Completed).is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert!(mgr.update_progress(&id, 100).is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert!(mgr.add_milestone(&id, "New step").is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert!(mgr.complete_milestone(&id, &milestone).is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert!(mgr.delete_goal(&id).is_err());
        assert_eq!(serde_json::to_value(&mgr.goals).unwrap(), before);
        assert_eq!(
            std::fs::read(dir.path().join("goals.json")).unwrap(),
            stored
        );
    }

    #[test]
    fn sub_goal_relationships_persist_together() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = manager(dir.path());
        let parent = mgr
            .create_goal("Parent", None, GoalPriority::Medium)
            .unwrap();
        let child = mgr.create_sub_goal(&parent, "Child", None).unwrap();
        let mut reloaded = manager(dir.path());
        reloaded.load().unwrap();
        assert_eq!(
            reloaded.get_goal(&parent).unwrap().sub_goals,
            vec![child.clone()]
        );
        assert_eq!(
            reloaded.get_goal(&child).unwrap().parent_id.as_deref(),
            Some(parent.as_str())
        );
    }
}
