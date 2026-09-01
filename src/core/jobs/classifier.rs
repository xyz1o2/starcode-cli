/// 作业分类器

use super::JobType;

/// 作业分类器
pub struct JobClassifier;

impl JobClassifier {
    /// 创建新的作业分类器
    pub fn new() -> Self {
        Self
    }

    /// 分类作业
    pub fn classify(&self, name: &str, input: &serde_json::Value) -> JobType {
        let name_lower = name.to_lowercase();

        // 根据名称分类
        if name_lower.contains("dream") {
            return JobType::Dream;
        }
        
        if name_lower.contains("agent") && name_lower.contains("remote") {
            return JobType::RemoteAgent;
        }
        
        if name_lower.contains("agent") {
            return JobType::LocalAgent;
        }
        
        if name_lower.contains("teammate") || name_lower.contains("swarm") {
            return JobType::InProcessTeammate;
        }
        
        if name_lower.contains("shell") || name_lower.contains("bash") {
            return JobType::LocalShell;
        }
        
        if name_lower.contains("workflow") {
            return JobType::LocalWorkflow;
        }
        
        if name_lower.contains("mcp") || name_lower.contains("monitor") {
            return JobType::MonitorMcp;
        }

        // 根据输入分类
        if let Some(job_type) = input.get("type").and_then(|v| v.as_str()) {
            match job_type.to_lowercase().as_str() {
                "dream" => return JobType::Dream,
                "agent" => return JobType::LocalAgent,
                "remote_agent" => return JobType::RemoteAgent,
                "teammate" => return JobType::InProcessTeammate,
                "shell" => return JobType::LocalShell,
                "workflow" => return JobType::LocalWorkflow,
                "mcp" => return JobType::MonitorMcp,
                _ => {}
            }
        }

        JobType::Custom(name.to_string())
    }
}
