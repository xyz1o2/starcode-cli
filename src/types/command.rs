/// 命令类型定义
///
/// 对标claude-code-main的src/types/command.ts
use serde::{Deserialize, Serialize};

/// 命令类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommandType {
    /// 斜杠命令
    Slash,
    /// 快捷键命令
    Shortcut,
    /// 菜单命令
    Menu,
    /// 上下文命令
    Context,
    /// 自动命令
    Auto,
}

/// 命令状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CommandStatus {
    /// 可用
    Available,
    /// 禁用
    Disabled,
    /// 执行中
    Running,
    /// 已完成
    Completed,
    /// 失败
    Failed,
}

/// 命令定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandDefinition {
    /// 命令名称
    pub name: String,
    /// 命令别名
    pub aliases: Vec<String>,
    /// 命令描述
    pub description: String,
    /// 命令类型
    pub command_type: CommandType,
    /// 参数定义
    pub parameters: Vec<CommandParameter>,
    /// 是否需要确认
    pub requires_confirmation: bool,
    /// 是否需要权限
    pub requires_permission: bool,
    /// 使用示例
    pub examples: Vec<String>,
}

/// 命令参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandParameter {
    /// 参数名称
    pub name: String,
    /// 参数描述
    pub description: String,
    /// 是否必需
    pub required: bool,
    /// 默认值
    pub default_value: Option<String>,
    /// 参数类型
    pub parameter_type: ParameterType,
    /// 有效值列表
    pub valid_values: Option<Vec<String>>,
}

/// 参数类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParameterType {
    /// 字符串
    String,
    /// 数字
    Number,
    /// 布尔
    Boolean,
    /// 文件路径
    FilePath,
    /// 目录路径
    DirectoryPath,
    /// 枚举
    Enum(Vec<String>),
    /// 数组
    Array(Box<ParameterType>),
}

/// 命令执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    /// 命令名称
    pub command: String,
    /// 是否成功
    pub success: bool,
    /// 输出内容
    pub output: Option<String>,
    /// 错误信息
    pub error: Option<String>,
    /// 执行时间（毫秒）
    pub duration_ms: u64,
    /// 侧效果
    pub side_effects: Vec<SideEffect>,
}

/// 副作用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideEffect {
    /// 副作用类型
    pub effect_type: SideEffectType,
    /// 描述
    pub description: String,
    /// 相关文件
    pub files: Vec<String>,
}

/// 副作用类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SideEffectType {
    /// 文件修改
    FileModified,
    /// 文件创建
    FileCreated,
    /// 文件删除
    FileDeleted,
    /// 配置更改
    ConfigChanged,
    /// 状态更改
    StateChanged,
    /// 网络请求
    NetworkRequest,
}
