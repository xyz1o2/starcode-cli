/// 作业模板

use super::JobType;
use serde::{Deserialize, Serialize};

/// 作业模板
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTemplate {
    /// 模板名称
    pub name: String,
    /// 模板描述
    pub description: String,
    /// 作业类型
    pub job_type: JobType,
    /// 默认输入
    pub default_input: serde_json::Value,
    /// 标签
    pub tags: Vec<String>,
}

/// 作业模板管理器
pub struct JobTemplates {
    templates: Vec<JobTemplate>,
}

impl JobTemplates {
    /// 创建新的作业模板管理器
    pub fn new() -> Self {
        let mut manager = Self {
            templates: Vec::new(),
        };
        
        manager.load_default_templates();
        manager
    }

    /// 加载默认模板
    fn load_default_templates(&mut self) {
        self.templates.push(JobTemplate {
            name: "dream".to_string(),
            description: "Dream task for memory consolidation".to_string(),
            job_type: JobType::Dream,
            default_input: serde_json::json!({}),
            tags: vec!["memory".to_string(), "dream".to_string()],
        });

        self.templates.push(JobTemplate {
            name: "local_agent".to_string(),
            description: "Local agent task execution".to_string(),
            job_type: JobType::LocalAgent,
            default_input: serde_json::json!({}),
            tags: vec!["agent".to_string()],
        });

        self.templates.push(JobTemplate {
            name: "shell_command".to_string(),
            description: "Shell command execution".to_string(),
            job_type: JobType::LocalShell,
            default_input: serde_json::json!({ "command": "" }),
            tags: vec!["shell".to_string(), "bash".to_string()],
        });

        self.templates.push(JobTemplate {
            name: "workflow".to_string(),
            description: "Workflow execution".to_string(),
            job_type: JobType::LocalWorkflow,
            default_input: serde_json::json!({ "workflow": "" }),
            tags: vec!["workflow".to_string()],
        });
    }

    /// 获取模板
    pub fn get_template(&self, name: &str) -> Option<&JobTemplate> {
        self.templates.iter().find(|t| t.name == name)
    }

    /// 获取所有模板
    pub fn get_all_templates(&self) -> &[JobTemplate] {
        &self.templates
    }

    /// 添加模板
    pub fn add_template(&mut self, template: JobTemplate) {
        self.templates.push(template);
    }
}
