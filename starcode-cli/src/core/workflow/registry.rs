/// 工作流注册表

use super::Workflow;

/// 工作流注册表
pub struct WorkflowRegistry {
    /// 注册的工作流
    workflows: std::collections::HashMap<String, Workflow>,
}

impl WorkflowRegistry {
    /// 创建新的工作流注册表
    pub fn new() -> Self {
        Self {
            workflows: std::collections::HashMap::new(),
        }
    }

    /// 注册工作流
    pub fn register(&mut self, workflow: Workflow) {
        self.workflows.insert(workflow.name.clone(), workflow);
    }

    /// 获取工作流
    pub fn get(&self, name: &str) -> Option<&Workflow> {
        self.workflows.get(name)
    }

    /// 列出所有工作流
    pub fn list(&self) -> Vec<&Workflow> {
        self.workflows.values().collect()
    }

    /// 删除工作流
    pub fn unregister(&mut self, name: &str) {
        self.workflows.remove(name);
    }
}
