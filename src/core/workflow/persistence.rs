/// 工作流持久化
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 可持久化的工作流上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedWorkflowContext {
    /// 工作流名称
    pub workflow_name: String,
    /// 当前步骤
    pub current_step: usize,
    /// 开始时间
    pub started_at: u64,
    /// 日志
    pub log: Vec<super::WorkflowLogEntry>,
}

/// 工作流持久化管理器
pub struct WorkflowPersistence {
    /// 存储目录
    storage_dir: PathBuf,
}

impl WorkflowPersistence {
    /// 创建新的工作流持久化管理器
    pub fn new(project_root: &std::path::Path) -> Self {
        let storage_dir = project_root.join(".starcode").join("workflows");
        Self { storage_dir }
    }

    /// 保存执行结果
    pub fn save_execution(&self, context: &super::WorkflowContext) -> Result<(), String> {
        // 确保目录存在
        std::fs::create_dir_all(&self.storage_dir)
            .map_err(|e| format!("Failed to create workflow dir: {}", e))?;

        let filename = format!("{}_{}.json", context.workflow_name, context.started_at);
        let filepath = self.storage_dir.join(filename);

        let persisted = PersistedWorkflowContext {
            workflow_name: context.workflow_name.clone(),
            current_step: context.current_step,
            started_at: context.started_at,
            log: context.log.clone(),
        };

        let content = serde_json::to_string_pretty(&persisted)
            .map_err(|e| format!("Failed to serialize context: {}", e))?;

        std::fs::write(&filepath, content)
            .map_err(|e| format!("Failed to write execution file: {}", e))?;

        Ok(())
    }

    /// 加载执行历史
    pub fn load_executions(
        &self,
        workflow_name: &str,
    ) -> Result<Vec<PersistedWorkflowContext>, String> {
        if !self.storage_dir.exists() {
            return Ok(Vec::new());
        }

        let mut executions = Vec::new();

        for entry in std::fs::read_dir(&self.storage_dir)
            .map_err(|e| format!("Failed to read workflow dir: {}", e))?
        {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(workflow_name))
                .unwrap_or(false)
            {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(context) = serde_json::from_str::<PersistedWorkflowContext>(&content)
                    {
                        executions.push(context);
                    }
                }
            }
        }

        Ok(executions)
    }
}
