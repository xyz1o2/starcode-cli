/// YOLO分类器类型定义

use serde::{Deserialize, Serialize};

/// 分类器输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierInput {
    /// 工具名称
    pub tool_name: String,
    /// 命令内容
    pub command: String,
    /// 工作目录
    pub working_directory: String,
    /// 上下文
    pub context: Option<String>,
}

/// 分类器行为
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ClassifierBehavior {
    /// 允许
    Allow,
    /// 需要确认
    Ask,
    /// 拒绝
    Deny,
}

/// 分类器结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierResult {
    /// 是否匹配规则
    pub matches: bool,
    /// 置信度
    pub confidence: String,
    /// 原因
    pub reason: String,
    /// 行为
    pub behavior: ClassifierBehavior,
}

/// 分类器描述
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierDescription {
    /// 描述文本
    pub description: String,
    /// 行为
    pub behavior: ClassifierBehavior,
    /// 优先级
    pub priority: u32,
}
