/// Hook事件定义

use serde::{Deserialize, Serialize};

/// Hook事件类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HookEventType {
    /// 工具执行前
    PreToolUse,
    /// 工具执行后
    PostToolUse,
    /// 工具执行失败后
    PostToolUseFailure,
    /// 权限被拒绝
    PermissionDenied,
    /// 权限请求
    PermissionRequest,
    /// 用户提交提示
    UserPromptSubmit,
    /// 会话开始
    SessionStart,
    /// 会话结束
    SessionEnd,
    /// 停止
    Stop,
    /// 停止失败
    StopFailure,
    /// 子代理开始
    SubagentStart,
    /// 子代理停止
    SubagentStop,
    /// 压缩前
    PreCompact,
    /// 压缩后
    PostCompact,
    /// 通知
    Notification,
    /// 设置
    Setup,
    /// 队友空闲
    TeammateIdle,
    /// 任务创建
    TaskCreated,
    /// 任务完成
    TaskCompleted,
    /// 引出
    Elicitation,
    /// 引出结果
    ElicitationResult,
    /// 配置变更
    ConfigChange,
    /// 工作目录变更
    CwdChanged,
    /// 文件变更
    FileChanged,
    /// 指令加载
    InstructionsLoaded,
}

/// Hook事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    /// 事件ID
    pub id: String,
    /// 事件类型
    pub event_type: HookEventType,
    /// 时间戳
    pub timestamp: i64,
    /// 事件数据
    pub data: serde_json::Value,
}

/// Hook执行事件
#[derive(Debug, Clone)]
pub enum HookExecutionEvent {
    /// Hook开始执行
    Started {
        hook_id: String,
        hook_name: String,
        hook_event: String,
    },
    /// Hook执行进度
    Progress {
        hook_id: String,
        hook_name: String,
        hook_event: String,
        stdout: String,
        stderr: String,
        output: String,
    },
    /// Hook执行响应
    Response {
        hook_id: String,
        hook_name: String,
        hook_event: String,
        output: String,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
        outcome: String,
    },
}
