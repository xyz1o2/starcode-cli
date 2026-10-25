//! 外部 harness 控制面 —— 文件 inbox 协议。
//!
//! # 为什么是文件而不是 socket
//!
//! 外部 harness 要能驱动一个正在跑的 StarCode 会话：发消息、批准工具、取消、
//! 查状态。文件协议的价值在于**零依赖、跨平台、没有鉴权面** —— 外部进程往
//! `.star/remote/inbox.jsonl` 追加一行 JSON 就行，StarCode 侧有后台轮询器在
//! 消费。出站方向走 `.star/remote/outbox.jsonl`（同样的追加语义）。
//!
//! 代价是延迟（轮询间隔，`runtime/background.rs` 里是 1s）和单向文件读写的
//! 竞态。这些对「CI 里跑一个 agent」这类场景够用；真要低延迟双向，仓库里
//! 另有 `core/bridge/` 的 WebSocket 实现（尚未接线）。
//!
//! # 协议
//!
//! v1（`starcode.remote.v1`）只有一个动作 `send_message`。v2
//! （`starcode.remote.v2`）增加了 `approve_tool` / `cancel` / `ping`，
//! 并引入 `request_id` 把请求和响应配对起来。
//!
//! **v1 仍然接受** —— 老脚本不该因为升级就断掉。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// 协议 v1：只支持 `send_message`。
pub const REMOTE_PROTOCOL_V1: &str = "starcode.remote.v1";
/// 协议 v2：支持全部动作，带 `request_id` 配对。
pub const REMOTE_PROTOCOL_V2: &str = "starcode.remote.v2";

/// 动作：发一条消息给会话。
pub const ACTION_SEND_MESSAGE: &str = "send_message";
/// 动作：批准（或拒绝）一次工具确认请求。
pub const ACTION_APPROVE_TOOL: &str = "approve_tool";
/// 动作：中止当前正在跑的回合。
pub const ACTION_CANCEL: &str = "cancel";
/// 动作：探活。回一条 `pong`，不带任何副作用。
pub const ACTION_PING: &str = "ping";

/// 全部受支持的协议版本。解析时按这个集合判定。
pub const SUPPORTED_PROTOCOLS: &[&str] = &[REMOTE_PROTOCOL_V1, REMOTE_PROTOCOL_V2];

/// 全部受支持的动作。
pub const SUPPORTED_ACTIONS: &[&str] = &[
    ACTION_SEND_MESSAGE,
    ACTION_APPROVE_TOOL,
    ACTION_CANCEL,
    ACTION_PING,
];

/// 外部 harness 投进来的请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRequest {
    /// 协议版本。缺省按 v1 处理（老脚本不写这个字段）。
    #[serde(default = "default_protocol")]
    pub protocol: String,
    /// 动作名。缺省按 `send_message` 处理。
    #[serde(default = "default_action")]
    pub action: String,
    /// 请求内容。`send_message` 是消息正文；`approve_tool` 是工具调用 id；
    /// `cancel` / `ping` 留空。
    #[serde(default)]
    pub message: String,
    /// 请求方标识，会回显在响应里。
    #[serde(default)]
    pub source: String,
    /// Unix 秒。用于让 harness 知道这条请求是什么时候写的。
    #[serde(default = "default_created_at")]
    pub created_at: i64,
    /// 请求 id。响应里原样带回，harness 靠它配对。
    #[serde(default)]
    pub request_id: Option<String>,
    /// `approve_tool` 用：批准还是拒绝。
    #[serde(default)]
    pub approved: Option<bool>,
    /// `approve_tool` 用：是不是「本会话总是允许」。
    #[serde(default)]
    pub always_allow: Option<bool>,
}

/// StarCode 回给外部 harness 的一条消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteResponse {
    /// 协议版本。回的是**实际生效**的版本 —— v1 请求也会回 v1，
    /// 免得老 harness 看到不认识的协议串。
    pub protocol: String,
    /// 对应请求的 `request_id`（请求没带就没有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// 动作名，与请求一致。
    pub action: String,
    /// 结果：`ok` / `error` / `pending`。
    pub status: String,
    /// 人可读的结果说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 动作相关的负载（例如 `status` 查询返回的会话状态）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// Unix 秒。
    pub created_at: i64,
}

impl RemoteResponse {
    pub fn ok(
        protocol: &str,
        request_id: Option<String>,
        action: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            protocol: protocol.to_string(),
            request_id,
            action: action.to_string(),
            status: "ok".to_string(),
            detail: Some(detail.into()),
            payload: None,
            created_at: Utc::now().timestamp(),
        }
    }

    pub fn error(
        protocol: &str,
        request_id: Option<String>,
        action: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            protocol: protocol.to_string(),
            request_id,
            action: action.to_string(),
            status: "error".to_string(),
            detail: Some(detail.into()),
            payload: None,
            created_at: Utc::now().timestamp(),
        }
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// 一批请求的消费结果。
#[derive(Debug, Clone, Default)]
pub struct RemoteDrainResult {
    /// 通过校验的请求。
    pub accepted: Vec<RemoteRequest>,
    /// 被拒的请求及原因（行号 + 原因）。
    pub rejected: Vec<String>,
}

fn default_protocol() -> String {
    REMOTE_PROTOCOL_V1.to_string()
}

fn default_action() -> String {
    ACTION_SEND_MESSAGE.to_string()
}

fn default_created_at() -> i64 {
    Utc::now().timestamp()
}

pub fn remote_dir(project_root: &Path) -> PathBuf {
    project_root.join(".star").join("remote")
}

pub fn inbox_file_path(project_root: &Path) -> PathBuf {
    remote_dir(project_root).join("inbox.jsonl")
}

/// 出站通道：StarCode → 外部 harness。
pub fn outbox_file_path(project_root: &Path) -> PathBuf {
    remote_dir(project_root).join("outbox.jsonl")
}

async fn ensure_remote_dir(project_root: &Path) -> Result<(), String> {
    let dir = remote_dir(project_root);
    if !dir.exists() {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("failed to create {}: {}", dir.display(), e))?;
    }
    Ok(())
}

pub async fn queue_message(
    project_root: &Path,
    message: String,
    source: Option<String>,
) -> Result<(), String> {
    // 空消息如果写进 inbox，每一轮 drain 都会被拒一次 —— 必须挡在写之前。
    if message.trim().is_empty() {
        return Err("message cannot be empty".to_string());
    }
    ensure_remote_dir(project_root).await?;
    let req = RemoteRequest {
        protocol: REMOTE_PROTOCOL_V2.to_string(),
        action: ACTION_SEND_MESSAGE.to_string(),
        message,
        source: source.unwrap_or_default(),
        created_at: default_created_at(),
        request_id: None,
        approved: None,
        always_allow: None,
    };
    append_request(project_root, &req).await
}

/// 往 inbox 追加一条请求（通用入口，v2 协议的字段都带上）。
pub async fn append_request(project_root: &Path, req: &RemoteRequest) -> Result<(), String> {
    ensure_remote_dir(project_root).await?;
    let path = inbox_file_path(project_root);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("failed to open {}: {}", path.display(), e))?;
    let line =
        serde_json::to_string(req).map_err(|e| format!("failed to serialize request: {e}"))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    file.write_all(b"\n")
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// 往 outbox 追加一条响应（StarCode → 外部 harness）。
///
/// 追加语义和多进程写同一个文件不同 —— 但外部 harness 是**单写者**（一个
/// inbox），StarCode 是**单写者**（一个 outbox），所以不需要文件锁。同一台机器
/// 上多个 StarCode 实例共享一个项目目录才会撞车，那种场景本来就该用
/// `--directory` 分开。
pub async fn append_response(project_root: &Path, resp: &RemoteResponse) -> Result<(), String> {
    ensure_remote_dir(project_root).await?;
    let path = outbox_file_path(project_root);
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("failed to open {}: {}", path.display(), e))?;
    let line =
        serde_json::to_string(resp).map_err(|e| format!("failed to serialize response: {e}"))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    file.write_all(b"\n")
        .await
        .map_err(|e| format!("failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// 读出 outbox 里的响应（给外部 harness 用的辅助；StarCode 自己不读）。
pub async fn read_outbox(project_root: &Path) -> Result<Vec<RemoteResponse>, String> {
    let path = outbox_file_path(project_root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    Ok(raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<RemoteResponse>(l).ok())
        .collect())
}

/// 请求是否需要非空的 `message`。
///
/// `send_message` 需要消息正文；`approve_tool` 需要工具调用 id；`cancel` 和
/// `ping` 不需要。
fn action_requires_message(action: &str) -> bool {
    matches!(action, ACTION_SEND_MESSAGE | ACTION_APPROVE_TOOL)
}

pub async fn queued_count(project_root: &Path) -> Result<usize, String> {
    let path = inbox_file_path(project_root);
    if !path.exists() {
        return Ok(0);
    }
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    Ok(content.lines().filter(|l| !l.trim().is_empty()).count())
}

pub async fn drain_requests(project_root: &Path) -> Result<RemoteDrainResult, String> {
    let path = inbox_file_path(project_root);
    if !path.exists() {
        return Ok(RemoteDrainResult::default());
    }

    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("failed to read {}: {}", path.display(), e))?;
    if raw.trim().is_empty() {
        return Ok(RemoteDrainResult::default());
    }

    tokio::fs::write(&path, "")
        .await
        .map_err(|e| format!("failed to truncate {}: {}", path.display(), e))?;

    let mut out = RemoteDrainResult::default();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed = serde_json::from_str::<RemoteRequest>(line);
        match parsed {
            Ok(req) => {
                if !SUPPORTED_PROTOCOLS.contains(&req.protocol.as_str()) {
                    out.rejected.push(format!(
                        "line {}: unsupported protocol `{}`",
                        idx + 1,
                        req.protocol
                    ));
                    continue;
                }
                if !SUPPORTED_ACTIONS.contains(&req.action.as_str()) {
                    out.rejected.push(format!(
                        "line {}: unsupported action `{}`",
                        idx + 1,
                        req.action
                    ));
                    continue;
                }
                if action_requires_message(&req.action) && req.message.trim().is_empty() {
                    out.rejected.push(format!(
                        "line {}: action `{}` requires a non-empty `message`",
                        idx + 1,
                        req.action
                    ));
                    continue;
                }
                out.accepted.push(req);
            }
            Err(err) => out
                .rejected
                .push(format!("line {}: invalid json ({})", idx + 1, err)),
        }
    }

    Ok(out)
}

pub fn protocol_example() -> &'static str {
    r#"{"protocol":"starcode.remote.v2","action":"send_message","source":"ci-bot","request_id":"r-1","message":"请检查 main 分支构建失败原因"}"#
}

/// 协议说明（给 `/remote protocol` 展示）。
pub fn protocol_help() -> String {
    format!(
        "### Inbox（harness → StarCode）\n\
         - 文件：`{{project}}/.star/remote/inbox.jsonl`，每行一个 JSON\n\
         - 协议版本：`{REMOTE_PROTOCOL_V2}`（`{REMOTE_PROTOCOL_V1}` 仍接受，但只有 send_message）\n\
         - 动作：\n\
         \x20 - `{ACTION_SEND_MESSAGE}` —— 发消息给会话（`message` 必填）\n\
         \x20 - `{ACTION_APPROVE_TOOL}` —— 批准/拒绝工具确认（`message` 填 tool_call_id，`approved` 填布尔）\n\
         \x20 - `{ACTION_CANCEL}` —— 中止当前回合\n\
         \x20 - `{ACTION_PING}` —— 探活，回一条 `pong`\n\
         - 可选字段：`request_id`（响应原样带回）、`source`（回显）、`always_allow`\n\n\
         ### Outbox（StarCode → harness）\n\
         - 文件：`{{project}}/.star/remote/outbox.jsonl`，每行一个 JSON\n\
         - 字段：`protocol` / `request_id` / `action` / `status`（ok|error|pending）/ `detail` / `payload`\n\n\
         ### 示例\n\
         ```json\n{}\n```",
        protocol_example()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_inbox(dir: &Path, lines: &[&str]) {
        let path = inbox_file_path(dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, lines.join("\n")).unwrap();
    }

    #[tokio::test]
    async fn v1_send_message_is_still_accepted() {
        let dir = std::env::temp_dir().join(format!("star-remote-v1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_inbox(
            &dir,
            &[r#"{"protocol":"starcode.remote.v1","action":"send_message","message":"hi"}"#],
        );

        let result = drain_requests(&dir).await.expect("drain");
        assert_eq!(result.accepted.len(), 1, "v1 必须继续可用");
        assert_eq!(result.accepted[0].message, "hi");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn v2_supports_every_action() {
        let dir = std::env::temp_dir().join(format!("star-remote-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_inbox(
            &dir,
            &[
                r#"{"protocol":"starcode.remote.v2","action":"send_message","message":"hi"}"#,
                r#"{"protocol":"starcode.remote.v2","action":"approve_tool","message":"call-1","approved":true}"#,
                r#"{"protocol":"starcode.remote.v2","action":"cancel"}"#,
                r#"{"protocol":"starcode.remote.v2","action":"ping"}"#,
            ],
        );

        let result = drain_requests(&dir).await.expect("drain");
        assert_eq!(result.accepted.len(), 4, "四个动作都该被接受");
        assert!(result.rejected.is_empty(), "{:?}", result.rejected);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn actions_that_need_a_message_reject_an_empty_one() {
        let dir = std::env::temp_dir().join(format!("star-remote-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_inbox(
            &dir,
            &[
                r#"{"protocol":"starcode.remote.v2","action":"send_message"}"#,
                r#"{"protocol":"starcode.remote.v2","action":"approve_tool"}"#,
            ],
        );

        let result = drain_requests(&dir).await.expect("drain");
        assert!(result.accepted.is_empty());
        assert_eq!(result.rejected.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unknown_protocol_and_action_are_rejected() {
        let dir = std::env::temp_dir().join(format!("star-remote-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_inbox(
            &dir,
            &[
                r#"{"protocol":"starcode.remote.v99","action":"send_message","message":"hi"}"#,
                r#"{"protocol":"starcode.remote.v2","action":"drop_database","message":"hi"}"#,
            ],
        );

        let result = drain_requests(&dir).await.expect("drain");
        assert!(result.accepted.is_empty());
        assert_eq!(result.rejected.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_inbox_is_emptied_after_draining() {
        let dir = std::env::temp_dir().join(format!("star-remote-drain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_inbox(
            &dir,
            &[r#"{"protocol":"starcode.remote.v2","action":"send_message","message":"hi"}"#],
        );

        drain_requests(&dir).await.expect("drain");
        let count = queued_count(&dir).await.expect("count");
        assert_eq!(count, 0, "消费过的请求不该被重复处理");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 空消息必须挡在写之前 —— 写进 inbox 的话每一轮 drain 都会拒一次。
    #[tokio::test]
    async fn queue_message_refuses_an_empty_message() {
        let dir = std::env::temp_dir().join(format!("star-remote-empty2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        let err = queue_message(&dir, "   ".to_string(), None)
            .await
            .expect_err("empty message must be refused");
        assert!(err.contains("message cannot be empty"), "{err}");
        assert!(
            !inbox_file_path(&dir).exists(),
            "被拒的消息不该留下 inbox 文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 协议说明必须提到**当前**的协议版本和全部动作 ——
    /// 说明写在 core 里，协议升级时它跟着走，不会在命令里写死旧版本。
    #[test]
    fn protocol_help_lists_every_action_and_both_directions() {
        let help = protocol_help();
        assert!(help.contains(REMOTE_PROTOCOL_V2), "{help}");
        assert!(
            help.contains(REMOTE_PROTOCOL_V1),
            "v1 仍被接受，说明里要点出来"
        );
        assert!(help.contains("inbox.jsonl"), "{help}");
        assert!(help.contains("outbox.jsonl"), "{help}");
        for action in SUPPORTED_ACTIONS {
            assert!(help.contains(action), "说明里缺动作 {action}: {help}");
        }
    }

    #[tokio::test]
    async fn responses_round_trip_through_the_outbox() {
        let dir = std::env::temp_dir().join(format!("star-remote-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let resp = RemoteResponse::ok(
            REMOTE_PROTOCOL_V2,
            Some("r-1".to_string()),
            ACTION_PING,
            "pong",
        );
        append_response(&dir, &resp).await.expect("append");

        let read = read_outbox(&dir).await.expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].request_id.as_deref(), Some("r-1"));
        assert_eq!(read[0].status, "ok");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
