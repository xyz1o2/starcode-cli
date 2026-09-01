/// 工具类型定义
///
/// 对标claude-code-main的src/types/tools.ts
use serde::{Deserialize, Serialize};

/// 工具类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolType {
    /// 内置工具
    Builtin,
    /// 插件工具
    Plugin,
    /// MCP工具
    Mcp,
    /// 自定义工具
    Custom,
}

/// 工具状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolStatus {
    /// 可用
    Available,
    /// 禁用
    Disabled,
    /// 执行中
    Running,
    /// 错误
    Error,
}

/// 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 工具类型
    pub tool_type: ToolType,
    /// 输入模式
    pub input_schema: serde_json::Value,
    /// 是否需要确认
    pub requires_confirmation: bool,
    /// 是否需要权限
    pub requires_permission: bool,
    /// 是否危险操作
    pub is_destructive: bool,
    /// 是否只读
    pub is_read_only: bool,
    /// 超时时间（毫秒）
    pub timeout_ms: u64,
    /// 标签
    pub tags: Vec<String>,
}

/// 工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// 调用ID
    pub id: String,
    /// 工具名称
    pub name: String,
    /// 输入参数
    pub input: serde_json::Value,
    /// 调用时间
    pub timestamp: i64,
}

/// 工具结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// 调用ID
    pub tool_call_id: String,
    /// 是否成功
    pub success: bool,
    /// 输出内容
    pub output: Option<String>,
    /// 错误信息
    pub error: Option<String>,
    /// 数据
    pub data: Option<serde_json::Value>,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
    /// 是否被取消
    pub cancelled: bool,
}

/// 工具权限
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermission {
    /// 工具名称
    pub tool: String,
    /// 是否允许
    pub allowed: bool,
    /// 权限级别
    pub level: PermissionLevel,
    /// 来源
    pub source: String,
}

/// 权限级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionLevel {
    /// 允许
    Allow,
    /// 拒绝
    Deny,
    /// 需要确认
    Ask,
    /// 只读
    ReadOnly,
}
