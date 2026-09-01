//! Swarm Mailbox 系统（对标 Claude Code 的 teammateMailbox）
//!
//! 实现团队成员间的异步消息传递：
//! - 按 teammate name 寻址
//! - 支持普通消息和结构化协议消息
//! - 原子写入（rename 保证一致性）
//! - 大小限制和清理策略

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// ── 配置常量 ──

/// 单条消息最大字符数
const MAX_MESSAGE_SIZE_BYTES: usize = 64 * 1024; // 64KB

/// mailbox 文件最大大小
const MAX_MAILBOX_SIZE_BYTES: usize = 4 * 1024 * 1024; // 4MB

/// 保留的最大消息数
const MAX_RETAINED_MESSAGES: usize = 1000;

/// 已读消息保留数
const MAX_READ_MESSAGES: usize = 200;

/// 未读协议消息保留数
const MAX_UNREAD_PROTOCOL_MESSAGES: usize = 2000;

// ── 消息类型 ──

/// Mailbox 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxMessage {
    /// 消息 ID
    pub id: String,
    /// 发送者名称
    pub from: String,
    /// 接收者名称（"*" 表示广播）
    pub to: String,
    /// 消息类型
    pub message_type: MessageType,
    /// 消息内容
    pub content: String,
    /// 消息摘要（用于 UI 显示）
    pub summary: Option<String>,
    /// 时间戳（毫秒）
    pub timestamp_ms: u128,
    /// 是否已读
    pub read: bool,
    /// 消息颜色（用于 UI 显示）
    pub color: Option<String>,
}

/// 消息类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageType {
    /// 普通文本消息
    PlainText,
    /// 广播消息
    Broadcast,
    /// 任务分配
    TaskAssignment,
    /// 权限请求
    PermissionRequest,
    /// 权限响应
    PermissionResponse,
    /// 计划审批请求
    PlanApprovalRequest,
    /// 计划审批响应
    PlanApprovalResponse,
    /// 关闭请求
    ShutdownRequest,
    /// 关闭批准
    ShutdownApproved,
    /// 关闭拒绝
    ShutdownRejected,
    /// 模式设置请求
    ModeSetRequest,
    /// 团队权限更新
    TeamPermissionUpdate,
    /// 空闲通知
    IdleNotification,
}

/// Mailbox 文件内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxFile {
    /// 团队名称
    pub team_name: String,
    /// 接收者名称
    pub agent_name: String,
    /// 消息列表
    pub messages: VecDeque<MailboxMessage>,
    /// 最后更新时间
    pub last_updated_ms: u128,
}

// ── Mailbox 管理器 ──

/// Mailbox 管理器
#[derive(Clone)]
pub struct MailboxManager {
    /// 基础目录 (~/.star/teams)
    base_dir: PathBuf,
}

impl MailboxManager {
    /// 创建新的 Mailbox 管理器
    pub fn new() -> Self {
        let base_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star")
            .join("teams");
        Self { base_dir }
    }

    /// 使用自定义目录创建
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// 获取 mailbox 文件路径
    fn mailbox_path(&self, team_name: &str, agent_name: &str) -> PathBuf {
        self.base_dir
            .join(team_name)
            .join("inboxes")
            .join(format!("{}.json", agent_name))
    }

    /// 读取 mailbox
    pub fn read_mailbox(
        &self,
        team_name: &str,
        agent_name: &str,
    ) -> Result<MailboxFile, MailboxError> {
        let path = self.mailbox_path(team_name, agent_name);
        if !path.exists() {
            return Ok(MailboxFile {
                team_name: team_name.to_string(),
                agent_name: agent_name.to_string(),
                messages: VecDeque::new(),
                last_updated_ms: now_ms(),
            });
        }

        let content =
            std::fs::read_to_string(&path).map_err(|e| MailboxError::IoError(e.to_string()))?;

        serde_json::from_str(&content).map_err(|e| MailboxError::ParseError(e.to_string()))
    }

    /// 写入 mailbox（原子写入：先写临时文件，再 rename）
    pub fn write_mailbox(&self, mailbox: &MailboxFile) -> Result<(), MailboxError> {
        let path = self.mailbox_path(&mailbox.team_name, &mailbox.agent_name);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MailboxError::IoError(e.to_string()))?;
        }

        // 序列化
        let content = serde_json::to_string_pretty(mailbox)
            .map_err(|e| MailboxError::SerializeError(e.to_string()))?;

        // 检查大小限制
        if content.len() > MAX_MAILBOX_SIZE_BYTES {
            return Err(MailboxError::MailboxFull {
                current_size: content.len(),
                max_size: MAX_MAILBOX_SIZE_BYTES,
            });
        }

        // 原子写入：先写临时文件，再 rename
        let temp_path = path.with_extension("json.tmp");
        std::fs::write(&temp_path, &content).map_err(|e| MailboxError::IoError(e.to_string()))?;
        std::fs::rename(&temp_path, &path).map_err(|e| MailboxError::IoError(e.to_string()))?;

        Ok(())
    }

    /// 发送消息
    pub fn send_message(
        &self,
        team_name: &str,
        to: &str,
        message: MailboxMessage,
    ) -> Result<(), MailboxError> {
        // 验证消息大小
        if message.content.len() > MAX_MESSAGE_SIZE_BYTES {
            return Err(MailboxError::MessageTooLarge {
                size: message.content.len(),
                max_size: MAX_MESSAGE_SIZE_BYTES,
            });
        }

        let mut mailbox = self.read_mailbox(team_name, to)?;

        // 添加消息
        mailbox.messages.push_back(message);
        mailbox.last_updated_ms = now_ms();

        // 清理旧消息
        self.cleanup_messages(&mut mailbox);

        self.write_mailbox(&mailbox)
    }

    /// 读取未读消息
    pub fn read_unread_messages(
        &self,
        team_name: &str,
        agent_name: &str,
    ) -> Result<Vec<MailboxMessage>, MailboxError> {
        let mailbox = self.read_mailbox(team_name, agent_name)?;
        Ok(mailbox
            .messages
            .iter()
            .filter(|m| !m.read)
            .cloned()
            .collect())
    }

    /// 标记消息为已读
    pub fn mark_as_read(
        &self,
        team_name: &str,
        agent_name: &str,
        message_id: &str,
    ) -> Result<(), MailboxError> {
        let mut mailbox = self.read_mailbox(team_name, agent_name)?;

        if let Some(msg) = mailbox.messages.iter_mut().find(|m| m.id == message_id) {
            msg.read = true;
        }

        mailbox.last_updated_ms = now_ms();
        self.write_mailbox(&mailbox)
    }

    /// 获取未读消息数量
    pub fn unread_count(&self, team_name: &str, agent_name: &str) -> Result<usize, MailboxError> {
        let mailbox = self.read_mailbox(team_name, agent_name)?;
        Ok(mailbox.messages.iter().filter(|m| !m.read).count())
    }

    /// 清理旧消息
    fn cleanup_messages(&self, mailbox: &mut MailboxFile) {
        // 分离已读和未读消息
        let mut read_messages: VecDeque<MailboxMessage> = VecDeque::new();
        let mut unread_messages: VecDeque<MailboxMessage> = VecDeque::new();
        let mut unread_protocol_messages: VecDeque<MailboxMessage> = VecDeque::new();

        for msg in mailbox.messages.drain(..) {
            if msg.read {
                read_messages.push_back(msg);
            } else if is_protocol_message(&msg.message_type) {
                unread_protocol_messages.push_back(msg);
            } else {
                unread_messages.push_back(msg);
            }
        }

        // 限制已读消息数量
        while read_messages.len() > MAX_READ_MESSAGES {
            read_messages.pop_front();
        }

        // 限制未读协议消息数量
        while unread_protocol_messages.len() > MAX_UNREAD_PROTOCOL_MESSAGES {
            unread_protocol_messages.pop_front();
        }

        // 合并消息
        mailbox.messages.extend(read_messages);
        mailbox.messages.extend(unread_messages);
        mailbox.messages.extend(unread_protocol_messages);

        // 总数限制
        while mailbox.messages.len() > MAX_RETAINED_MESSAGES {
            mailbox.messages.pop_front();
        }
    }
}

// ── 辅助函数 ──

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn is_protocol_message(msg_type: &MessageType) -> bool {
    matches!(
        msg_type,
        MessageType::TaskAssignment
            | MessageType::PermissionRequest
            | MessageType::PermissionResponse
            | MessageType::PlanApprovalRequest
            | MessageType::PlanApprovalResponse
            | MessageType::ShutdownRequest
            | MessageType::ShutdownApproved
            | MessageType::ShutdownRejected
            | MessageType::ModeSetRequest
            | MessageType::TeamPermissionUpdate
    )
}

/// 生成消息 ID
pub fn generate_message_id() -> String {
    let timestamp = now_ms();
    let random = uuid::Uuid::new_v4().to_string();
    format!("msg_{}_{}", timestamp, &random[..8])
}

// ── 错误类型 ──

#[derive(Debug)]
pub enum MailboxError {
    IoError(String),
    ParseError(String),
    SerializeError(String),
    MessageTooLarge {
        size: usize,
        max_size: usize,
    },
    MailboxFull {
        current_size: usize,
        max_size: usize,
    },
}

impl std::fmt::Display for MailboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MailboxError::IoError(msg) => write!(f, "Mailbox IO error: {}", msg),
            MailboxError::ParseError(msg) => write!(f, "Mailbox parse error: {}", msg),
            MailboxError::SerializeError(msg) => write!(f, "Mailbox serialize error: {}", msg),
            MailboxError::MessageTooLarge { size, max_size } => {
                write!(f, "Message too large: {} bytes (max: {})", size, max_size)
            }
            MailboxError::MailboxFull {
                current_size,
                max_size,
            } => {
                write!(
                    f,
                    "Mailbox full: {} bytes (max: {})",
                    current_size, max_size
                )
            }
        }
    }
}

impl std::error::Error for MailboxError {}

// ── SendMessage 工具 ──

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};

/// SendMessage 工具：向团队成员发送消息
#[derive(Clone)]
pub struct SendMessageTool {
    mailbox: MailboxManager,
}

impl SendMessageTool {
    pub fn new() -> Self {
        Self {
            mailbox: MailboxManager::new(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SendMessageParams {
    pub to: String,
    pub message: String,
    pub summary: Option<String>,
    pub team_name: Option<String>,
}

pub struct SendMessageInvocation {
    params: SendMessageParams,
    mailbox: MailboxManager,
}

impl ToolInvocation for SendMessageInvocation {
    fn get_description(&self) -> String {
        format!("Send message to {}", self.params.to)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        let mailbox = MailboxManager::new();
        Box::pin(async move {
            let team_name = params.team_name.unwrap_or_else(|| "default".to_string());

            let message = MailboxMessage {
                id: generate_message_id(),
                from: "lead".to_string(), // 应该从上下文获取
                to: params.to.clone(),
                message_type: if params.to == "*" {
                    MessageType::Broadcast
                } else {
                    MessageType::PlainText
                },
                content: params.message,
                summary: params.summary,
                timestamp_ms: now_ms(),
                read: false,
                color: None,
            };

            mailbox.send_message(&team_name, &params.to, message)?;

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Message sent to '{}' in team '{}'",
                    params.to, team_name
                )),
                return_display: Some(format!("Message sent to {}", params.to)),
                output: String::new(),
                error: None,
                data: Some(serde_json::json!({
                    "to": params.to,
                    "team": team_name,
                    "sent": true
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }
    fn display_name(&self) -> &str {
        "SendMessage"
    }
    fn description(&self) -> &str {
        "向团队成员发送消息。用于多Agent协作中的通信。(Send a message to a team member. Used for communication in multi-agent collaboration.)"
    }
    fn kind(&self) -> Kind {
        Kind::Other
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "接收者名称，'*' 表示广播 (Recipient name, '*' for broadcast)"
                },
                "message": {
                    "type": "string",
                    "description": "消息内容 (Message content)"
                },
                "summary": {
                    "type": "string",
                    "description": "消息摘要，用于 UI 显示 (Message summary for UI display)"
                },
                "team_name": {
                    "type": "string",
                    "description": "团队名称，默认 'default' (Team name, defaults to 'default')"
                }
            },
            "required": ["to", "message"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SendMessageParams = serde_json::from_value(params)?;
        Ok(Box::new(SendMessageInvocation {
            params,
            mailbox: MailboxManager::new(),
        }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mailbox_send_and_receive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mailbox = MailboxManager::with_base_dir(temp_dir.path().to_path_buf());

        let msg = MailboxMessage {
            id: generate_message_id(),
            from: "lead".to_string(),
            to: "researcher".to_string(),
            message_type: MessageType::PlainText,
            content: "Hello!".to_string(),
            summary: Some("Greeting".to_string()),
            timestamp_ms: now_ms(),
            read: false,
            color: None,
        };

        mailbox
            .send_message("test-team", "researcher", msg)
            .unwrap();

        let unread = mailbox
            .read_unread_messages("test-team", "researcher")
            .unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].content, "Hello!");
    }

    #[test]
    fn test_mailbox_mark_as_read() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mailbox = MailboxManager::with_base_dir(temp_dir.path().to_path_buf());

        let msg = MailboxMessage {
            id: "test-msg-1".to_string(),
            from: "lead".to_string(),
            to: "researcher".to_string(),
            message_type: MessageType::PlainText,
            content: "Hello!".to_string(),
            summary: None,
            timestamp_ms: now_ms(),
            read: false,
            color: None,
        };

        mailbox
            .send_message("test-team", "researcher", msg)
            .unwrap();
        mailbox
            .mark_as_read("test-team", "researcher", "test-msg-1")
            .unwrap();

        let unread = mailbox
            .read_unread_messages("test-team", "researcher")
            .unwrap();
        assert_eq!(unread.len(), 0);
    }
}
