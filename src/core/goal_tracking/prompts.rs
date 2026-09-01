/// 目标提示词
use super::Goal;

/// 目标提示词管理器
pub struct GoalPrompts;

impl GoalPrompts {
    /// 创建新的目标提示词管理器
    pub fn new() -> Self {
        Self
    }

    /// 生成目标提示词
    pub fn generate_goal_prompt(&self, goal: &Goal) -> String {
        let mut prompt = format!("## Current Goal: {}\n\n", goal.title);

        if let Some(description) = &goal.description {
            prompt.push_str(&format!("**Description:** {}\n\n", description));
        }

        prompt.push_str(&format!("**Status:** {:?}\n", goal.status));
        prompt.push_str(&format!("**Priority:** {:?}\n", goal.priority));
        prompt.push_str(&format!("**Progress:** {}%\n", goal.progress));

        if !goal.milestones.is_empty() {
            prompt.push_str("\n### Milestones\n");
            for milestone in &goal.milestones {
                let status = if milestone.completed { "✓" } else { "○" };
                prompt.push_str(&format!("- {} {}\n", status, milestone.title));
            }
        }

        if !goal.sub_goals.is_empty() {
            prompt.push_str("\n### Sub-goals\n");
            for sub_goal_id in &goal.sub_goals {
                prompt.push_str(&format!("- {}\n", sub_goal_id));
            }
        }

        prompt
    }
}
