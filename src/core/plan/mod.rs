//! Plan Manager 模块
//!
//! 对标 Claude Code 的 Plan Mode 增强功能：
//! - 计划持久化（保存到 .star/plans/）
//! - 计划模板
//! - 计划历史
//! - 计划版本控制

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ── 计划模型 ──

/// 计划状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStatus {
    /// 草稿
    Draft,
    /// 已审批
    Approved,
    /// 执行中
    InProgress,
    /// 已完成
    Completed,
    /// 已取消
    Cancelled,
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanStatus::Draft => write!(f, "draft"),
            PlanStatus::Approved => write!(f, "approved"),
            PlanStatus::InProgress => write!(f, "in_progress"),
            PlanStatus::Completed => write!(f, "completed"),
            PlanStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// 计划任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    /// 任务 ID
    pub id: String,
    /// 任务描述
    pub description: String,
    /// 是否已完成
    pub completed: bool,
    /// 子任务
    pub subtasks: Vec<PlanTask>,
}

/// 计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// 计划 ID
    pub id: String,
    /// 计划标题
    pub title: String,
    /// 计划内容（Markdown）
    pub content: String,
    /// 计划状态
    pub status: PlanStatus,
    /// 创建时间（毫秒）
    pub created_at_ms: u128,
    /// 更新时间（毫秒）
    pub updated_at_ms: u128,
    /// 任务列表
    pub tasks: Vec<PlanTask>,
    /// 标签
    pub tags: Vec<String>,
    /// 关联的会话 ID
    pub session_id: Option<String>,
    /// 计划版本
    pub version: u32,
}

impl Plan {
    /// 创建新计划
    pub fn new(title: String, content: String) -> Self {
        let now = now_ms();
        Self {
            id: generate_plan_id(),
            title,
            content,
            status: PlanStatus::Draft,
            created_at_ms: now,
            updated_at_ms: now,
            tasks: Vec::new(),
            tags: Vec::new(),
            session_id: None,
            version: 1,
        }
    }

    /// 更新计划内容
    pub fn update_content(&mut self, content: String) {
        self.content = content;
        self.updated_at_ms = now_ms();
        self.version += 1;
    }

    /// 更新计划状态
    pub fn set_status(&mut self, status: PlanStatus) {
        self.status = status;
        self.updated_at_ms = now_ms();
    }

    /// 添加任务
    pub fn add_task(&mut self, task: PlanTask) {
        self.tasks.push(task);
        self.updated_at_ms = now_ms();
    }

    /// 标记任务完成
    pub fn complete_task(&mut self, task_id: &str) -> bool {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.completed = true;
            self.updated_at_ms = now_ms();
            true
        } else {
            false
        }
    }

    /// 检查所有任务是否完成
    pub fn all_tasks_completed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.completed)
    }

    /// 获取完成进度
    pub fn progress(&self) -> (usize, usize) {
        let completed = self.tasks.iter().filter(|t| t.completed).count();
        (completed, self.tasks.len())
    }
}

// ── Plan Manager ──

/// Plan Manager
pub struct PlanManager {
    /// 存储目录
    storage_dir: PathBuf,
}

impl PlanManager {
    /// 创建新的 Plan Manager
    pub fn new() -> Self {
        let storage_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star")
            .join("plans");
        Self { storage_dir }
    }

    /// 使用自定义目录创建
    pub fn with_storage_dir(storage_dir: PathBuf) -> Self {
        Self { storage_dir }
    }

    /// 获取计划文件路径
    fn plan_path(&self, plan_id: &str) -> PathBuf {
        self.storage_dir.join(format!("{}.json", plan_id))
    }

    /// 保存计划
    pub fn save_plan(&self, plan: &Plan) -> Result<(), PlanError> {
        let path = self.plan_path(&plan.id);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| PlanError::IoError(e.to_string()))?;
        }

        let content = serde_json::to_string_pretty(plan)
            .map_err(|e| PlanError::SerializeError(e.to_string()))?;

        std::fs::write(&path, content)
            .map_err(|e| PlanError::IoError(e.to_string()))?;

        Ok(())
    }

    /// 加载计划
    pub fn load_plan(&self, plan_id: &str) -> Result<Plan, PlanError> {
        let path = self.plan_path(plan_id);

        if !path.exists() {
            return Err(PlanError::NotFound(plan_id.to_string()));
        }

        let content = std::fs::read_to_string(&path)
            .map_err(|e| PlanError::IoError(e.to_string()))?;

        serde_json::from_str(&content)
            .map_err(|e| PlanError::ParseError(e.to_string()))
    }

    /// 列出所有计划
    pub fn list_plans(&self) -> Result<Vec<Plan>, PlanError> {
        if !self.storage_dir.exists() {
            return Ok(Vec::new());
        }

        let mut plans = Vec::new();

        for entry in std::fs::read_dir(&self.storage_dir)
            .map_err(|e| PlanError::IoError(e.to_string()))?
        {
            let entry = entry.map_err(|e| PlanError::IoError(e.to_string()))?;
            let path = entry.path();

            if path.extension().map(|ext| ext == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(plan) = serde_json::from_str::<Plan>(&content) {
                        plans.push(plan);
                    }
                }
            }
        }

        // 按更新时间排序（最新的在前）
        plans.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

        Ok(plans)
    }

    /// 删除计划
    pub fn delete_plan(&self, plan_id: &str) -> Result<(), PlanError> {
        let path = self.plan_path(plan_id);

        if !path.exists() {
            return Err(PlanError::NotFound(plan_id.to_string()));
        }

        std::fs::remove_file(&path)
            .map_err(|e| PlanError::IoError(e.to_string()))?;

        Ok(())
    }

    /// 从 Markdown 内容解析任务
    pub fn parse_tasks_from_markdown(content: &str) -> Vec<PlanTask> {
        let mut tasks = Vec::new();
        let mut task_id = 1;

        for line in content.lines() {
            let trimmed = line.trim();

            // 匹配任务列表格式：- [ ] 或 - [x] 或 * [ ] 或 * [x]
            if trimmed.starts_with("- [ ]") || trimmed.starts_with("- [x]")
                || trimmed.starts_with("* [ ]") || trimmed.starts_with("* [x]")
            {
                let completed = trimmed.contains("[x]");
                let description = trimmed
                    .trim_start_matches("- [ ]")
                    .trim_start_matches("- [x]")
                    .trim_start_matches("* [ ]")
                    .trim_start_matches("* [x]")
                    .trim()
                    .to_string();

                tasks.push(PlanTask {
                    id: format!("task-{}", task_id),
                    description,
                    completed,
                    subtasks: Vec::new(),
                });
                task_id += 1;
            }
        }

        tasks
    }
}

// ── 辅助函数 ──

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn generate_plan_id() -> String {
    let timestamp = now_ms();
    let random = uuid::Uuid::new_v4().to_string();
    format!("plan_{}_{}", timestamp, &random[..8])
}

// ── 错误类型 ──

#[derive(Debug)]
pub enum PlanError {
    IoError(String),
    ParseError(String),
    SerializeError(String),
    NotFound(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::IoError(msg) => write!(f, "Plan IO error: {}", msg),
            PlanError::ParseError(msg) => write!(f, "Plan parse error: {}", msg),
            PlanError::SerializeError(msg) => write!(f, "Plan serialize error: {}", msg),
            PlanError::NotFound(id) => write!(f, "Plan not found: {}", id),
        }
    }
}

impl std::error::Error for PlanError {}

// ── 计划模板 ──

/// 计划模板
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTemplate {
    /// 模板名称
    pub name: String,
    /// 模板描述
    pub description: String,
    /// 模板内容（Markdown）
    pub content: String,
    /// 标签
    pub tags: Vec<String>,
}

/// 内置计划模板
pub fn builtin_templates() -> Vec<PlanTemplate> {
    vec![
        PlanTemplate {
            name: "feature".to_string(),
            description: "Feature implementation plan".to_string(),
            content: r#"# Feature: [Feature Name]

## Overview
Brief description of the feature.

## Requirements
- [ ] Requirement 1
- [ ] Requirement 2
- [ ] Requirement 3

## Implementation Steps
1. **Step 1**: Description
   - [ ] Sub-task 1.1
   - [ ] Sub-task 1.2
2. **Step 2**: Description
   - [ ] Sub-task 2.1
   - [ ] Sub-task 2.2

## Testing
- [ ] Unit tests
- [ ] Integration tests
- [ ] Manual testing

## Documentation
- [ ] Update README
- [ ] Add inline comments
"#.to_string(),
            tags: vec!["feature".to_string()],
        },
        PlanTemplate {
            name: "bugfix".to_string(),
            description: "Bug fix plan".to_string(),
            content: r#"# Bug Fix: [Bug Description]

## Problem
Description of the bug.

## Root Cause
Analysis of the root cause.

## Solution
Description of the fix.

## Steps
- [ ] Step 1: [Description]
- [ ] Step 2: [Description]
- [ ] Step 3: [Description]

## Verification
- [ ] Reproduce the bug
- [ ] Apply the fix
- [ ] Verify the fix
- [ ] Run existing tests
- [ ] Add new tests if needed
"#.to_string(),
            tags: vec!["bugfix".to_string()],
        },
        PlanTemplate {
            name: "refactor".to_string(),
            description: "Refactoring plan".to_string(),
            content: r#"# Refactor: [Component Name]

## Current State
Description of the current implementation.

## Problems
- Problem 1
- Problem 2
- Problem 3

## Proposed Changes
Description of the refactoring.

## Steps
- [ ] Step 1: [Description]
- [ ] Step 2: [Description]
- [ ] Step 3: [Description]

## Impact Analysis
- Affected components
- Breaking changes
- Migration steps

## Testing
- [ ] Existing tests pass
- [ ] New tests added
- [ ] Performance verified
"#.to_string(),
            tags: vec!["refactor".to_string()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_creation() {
        let plan = Plan::new("Test Plan".to_string(), "Content".to_string());
        assert_eq!(plan.title, "Test Plan");
        assert_eq!(plan.status, PlanStatus::Draft);
        assert_eq!(plan.version, 1);
    }

    #[test]
    fn test_plan_progress() {
        let mut plan = Plan::new("Test".to_string(), "".to_string());
        plan.add_task(PlanTask {
            id: "1".to_string(),
            description: "Task 1".to_string(),
            completed: false,
            subtasks: Vec::new(),
        });
        plan.add_task(PlanTask {
            id: "2".to_string(),
            description: "Task 2".to_string(),
            completed: false,
            subtasks: Vec::new(),
        });

        assert_eq!(plan.progress(), (0, 2));

        plan.complete_task("1");
        assert_eq!(plan.progress(), (1, 2));
    }

    #[test]
    fn test_parse_tasks_from_markdown() {
        let content = r#"# Plan
- [ ] Task 1
- [x] Task 2
- [ ] Task 3
"#;
        let tasks = PlanManager::parse_tasks_from_markdown(content);
        assert_eq!(tasks.len(), 3);
        assert!(!tasks[0].completed);
        assert!(tasks[1].completed);
        assert!(!tasks[2].completed);
    }
}
