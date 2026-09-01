/// 权限类型定义
///
/// 对标claude-code-main的src/types/permissions.ts
use serde::{Deserialize, Serialize};

/// 权限模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionMode {
    /// 自动模式
    Auto,
    /// 手动模式
    Manual,
    /// 计划模式
    Plan,
    /// 只读模式
    ReadOnly,
}

/// 权限结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionResult {
    /// 是否允许
    pub allowed: bool,
    /// 原因
    pub reason: Option<String>,
    /// 是否记住选择
    pub remember: bool,
    /// 权限规则
    pub rule: Option<PermissionRule>,
}

/// 权限规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// 规则ID
    pub id: String,
    /// 规则名称
    pub name: String,
    /// 规则描述
    pub description: String,
    /// 工具模式
    pub tool_pattern: String,
    /// 动作
    pub action: PermissionAction,
    /// 优先级
    pub priority: i32,
    /// 是否启用
    pub enabled: bool,
}

/// 权限动作
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionAction {
    /// 允许
    Allow,
    /// 拒绝
    Deny,
    /// 需要确认
    Ask,
}

/// 权限上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionContext {
    /// 工具名称
    pub tool: String,
    /// 输入参数
    pub input: serde_json::Value,
    /// 当前目录
    pub working_directory: String,
    /// 用户ID
    pub user_id: Option<String>,
    /// 会话ID
    pub session_id: String,
}
