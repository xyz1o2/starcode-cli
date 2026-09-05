/// 技能观察器
///
/// 观察用户行为并记录观察结果
use serde::{Deserialize, Serialize};

/// 观察类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ObservationType {
    /// 工具调用
    ToolCall,
    /// 命令执行
    CommandExecution,
    /// 文件操作
    FileOperation,
    /// 错误修复
    ErrorFix,
    /// 代码重构
    Refactoring,
    /// 测试编写
    TestWriting,
}

/// 观察结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    /// 观察ID
    pub id: String,
    /// 观察类型
    pub observation_type: ObservationType,
    /// 时间戳
    pub timestamp: i64,
    /// 上下文
    pub context: String,
    /// 动作
    pub action: String,
    /// 结果
    pub result: String,
    /// 是否成功
    pub success: bool,
    /// 相关文件
    pub files: Vec<String>,
    /// 标签
    pub tags: Vec<String>,
}

/// 技能观察器
pub struct SkillObserver {
    /// 观察历史
    observations: Vec<Observation>,
    /// 最大观察数
    max_observations: usize,
}

impl SkillObserver {
    /// 创建新的技能观察器
    pub fn new() -> Self {
        Self {
            observations: Vec::new(),
            max_observations: 1000,
        }
    }

    /// 记录观察
    pub fn record(&mut self, observation: Observation) {
        self.observations.push(observation);

        // 限制观察历史大小
        if self.observations.len() > self.max_observations {
            self.observations.remove(0);
        }
    }

    /// 获取待处理的观察
    pub fn get_pending_observations(&self) -> Vec<&Observation> {
        // 返回最近的观察
        self.observations.iter().rev().take(10).collect()
    }

    /// 获取所有观察
    pub fn get_all_observations(&self) -> &[Observation] {
        &self.observations
    }

    /// 按类型获取观察
    pub fn get_observations_by_type(&self, obs_type: &ObservationType) -> Vec<&Observation> {
        self.observations
            .iter()
            .filter(|obs| {
                std::mem::discriminant(&obs.observation_type) == std::mem::discriminant(obs_type)
            })
            .collect()
    }

    /// 清理旧观察
    pub fn cleanup_old_observations(&mut self, max_age_seconds: i64) {
        let cutoff = chrono::Utc::now().timestamp() - max_age_seconds;
        self.observations.retain(|obs| obs.timestamp > cutoff);
    }
}
