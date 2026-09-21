//! `TodoWrite` 工具 —— 对标 Claude Code 的 TodoWriteTool。
//!
//! 参照 `study_or_copy_projects/claude-code-main/packages/builtin-tools/src/tools/TodoWriteTool`：
//!
//! - 输入是**强类型的 todos 数组** `{content, status, activeForm}`，`deny_unknown_fields`
//!   严格校验。形状不对就报错回给模型让它自己重试 —— 不做任何"字符串里解 JSON"
//!   的兜底（CC 的 strictObject 哲学：宁可报错也不猜）。
//! - 每次调用**整表替换**当前清单；全部 completed 时清单清空（对齐 CC 的
//!   `newTodos = allDone ? [] : todos`）。
//! - 持久化仍复用 `TaskManager`/`TaskGraph`（`.star/tasks*.json`），task panel 的
//!   `reload()` 路径因此保持不变；`activeForm` 存进 [`TaskNode::active_form`]。

use crate::core::config::Config;
use crate::core::tasks::manager::TaskManager;
use crate::core::tasks::models::{TaskGraph, TaskNode, TaskStatus};
use crate::core::tools::tools::{BaseDeclarativeTool, ToolInvocation, ToolLocation, ToolResult};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// CC 只允许三种状态；Blocked/Skipped 属于共享任务清单（task_* 工具族），不进 checklist。
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TodoItemInput {
    /// 祈使句："Run tests"
    pub content: String,
    pub status: TodoStatus,
    /// 进行时："Running tests"（面板/转圈在 in_progress 时显示）。
    /// schema 按 CC 声明 camelCase `activeForm`，这里两种拼法都收。
    #[serde(alias = "activeForm")]
    pub active_form: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct TodoWriteParams {
    pub todos: Vec<TodoItemInput>,
}

/// 解析并校验入参。zod 的 `min(1)` 对齐：content / activeForm 都不能是空白串。
pub(crate) fn parse_todo_write_params(params: &Value) -> Result<TodoWriteParams, String> {
    let parsed: TodoWriteParams = serde_json::from_value(params.clone())
        .map_err(|e| format!("invalid TodoWrite parameters: {}", e))?;
    for item in &parsed.todos {
        if item.content.trim().is_empty() {
            return Err("invalid TodoWrite parameters: todo content cannot be empty".to_string());
        }
        if item.active_form.trim().is_empty() {
            return Err(
                "invalid TodoWrite parameters: todo activeForm cannot be empty".to_string(),
            );
        }
    }
    Ok(parsed)
}

/// 生成替换后的任务图。全部 completed → 空图（清单完成即清空）。
///
/// `prev` 是落盘文件里的旧图。TodoWrite 每次整表替换，节点 id 全换新的，
/// 但完成时间戳要尽量延续：面板的 30s TTL（`is_completed_expired`）和
/// "最近完成排最前"的排序都读 `completed_at`，如果每次重写都盖成 now，
/// 模型只要不停地把已完成项原样发回来，它们就永远不过期，清单被 ✔ 顶满
/// 下不去。按 content（= 落库后的 title）匹配旧节点，命中就沿用旧时间戳。
fn next_todo_graph(items: &[TodoItemInput], prev: &TaskGraph) -> TaskGraph {
    let mut graph = TaskGraph::new();
    if items
        .iter()
        .all(|item| item.status == TodoStatus::Completed)
    {
        return graph;
    }
    // content → 最近一次完成时间。重名条目取最旧的，保持"更早完成"的语义。
    let mut prev_completed_at: HashMap<&str, DateTime<Utc>> = HashMap::new();
    for n in prev.nodes.values() {
        if n.status != TaskStatus::Completed {
            continue;
        }
        let Some(t) = n.completed_at else {
            continue;
        };
        prev_completed_at
            .entry(n.title.as_str())
            .and_modify(|old| {
                if t < *old {
                    *old = t;
                }
            })
            .or_insert(t);
    }

    for item in items {
        let mut node = TaskNode::new(item.content.clone());
        node.status = match item.status {
            TodoStatus::Pending => TaskStatus::Pending,
            TodoStatus::InProgress => TaskStatus::InProgress,
            TodoStatus::Completed => TaskStatus::Completed,
        };
        // 只有 completed 盖时间戳；回到 pending/in_progress 要清掉，否则
        // 旧的 completed_at 会让它被 TTL 误删（toggle_status 同理）。
        node.completed_at = if node.status == TaskStatus::Completed {
            Some(
                prev_completed_at
                    .get(item.content.as_str())
                    .copied()
                    .unwrap_or_else(Utc::now),
            )
        } else {
            None
        };
        node.active_form = Some(item.active_form.clone());
        graph.root_ids.push(node.id.clone());
        graph.nodes.insert(node.id.clone(), node);
    }
    graph
}

pub struct TodoWriteTool {
    name: String,
    config: Arc<Config>,
    session_id: Option<String>,
    team_name: Option<String>,
}

impl TodoWriteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            name: "TodoWrite".to_string(),
            config,
            session_id: None,
            team_name: None,
        }
    }

    /// 设置会话ID以启用会话隔离
    pub fn with_session_id(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// 设置团队名称以启用团队隔离
    pub fn with_team_name(mut self, team_name: String) -> Self {
        self.team_name = Some(team_name);
        self
    }

    /// 按 session > team > workspace 的优先级选落盘文件（与旧 Todo 工具一致）
    fn task_file(&self) -> PathBuf {
        if let Some(team) = &self.team_name {
            TaskManager::task_file_for_team(self.config.working_dir(), team)
        } else if let Some(session) = &self.session_id {
            TaskManager::task_file_for_session(self.config.working_dir(), session)
        } else {
            TaskManager::task_file_for_workspace(self.config.working_dir())
        }
    }
}

impl BaseDeclarativeTool for TodoWriteTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn display_name(&self) -> &str {
        "Todo Write"
    }

    fn description(&self) -> &str {
        "Update the todo list for the current session. To be used proactively and often to track progress and pending tasks. Make sure that at least one task is in_progress at all times. Always provide both content (imperative) and activeForm (present continuous) for each task."
    }

    fn kind(&self) -> crate::core::tools::tools::Kind {
        crate::core::tools::tools::Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["todos"],
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The updated todo list",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["content", "status", "activeForm"],
                        "properties": {
                            "content": {
                                "type": "string",
                                "minLength": 1,
                                "description": "The imperative form describing what needs to be done (e.g., \"Run tests\")"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Task state; only ONE task may be in_progress at a time"
                            },
                            "activeForm": {
                                "type": "string",
                                "minLength": 1,
                                "description": "Present continuous form shown during execution (e.g., \"Running tests\")"
                            }
                        }
                    }
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params = parse_todo_write_params(&params)
            .map_err(|e| Box::<dyn std::error::Error + Send + Sync>::from(e))?;
        Ok(Box::new(TodoWriteInvocation {
            params,
            path: self.task_file(),
            workspace: self.config.working_dir().clone(),
        }))
    }
}

pub struct TodoWriteInvocation {
    params: TodoWriteParams,
    path: PathBuf,
    workspace: PathBuf,
}

impl ToolInvocation for TodoWriteInvocation {
    fn get_description(&self) -> String {
        format!("Update todo list ({} items)", self.params.todos.len())
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        Vec::new()
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let items = self.params.todos.clone();
        let count = items.len();
        let path = self.path.clone();
        let workspace = self.workspace.clone();

        Box::pin(async move {
            let output = tokio::task::spawn_blocking(move || {
                let mut manager =
                    TaskManager::load_from_file(&path).unwrap_or_else(|_| TaskManager::new());
                // 整表替换前把旧图交给 next_todo_graph：旧节点的 completed_at 按
                // content 沿用，面板的 30s TTL 不会每次重写都被重置。
                manager.graph = next_todo_graph(&items, &manager.graph);
                manager
                    .save_to_file(&path)
                    .map_err(|e| format!("Failed to save todo list: {}", e))?;
                Ok::<(), String>(())
            })
            .await
            .map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as Box<dyn std::error::Error>
            })?
            .map_err(|e| {
                Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
                    as Box<dyn std::error::Error>
            })?;

            // 对齐 CC 的 tool_result 文案：确认写入 + 敦促继续用清单跟踪进度
            Ok(ToolResult {
                llm_content: None,
                return_display: None,
                output: "Todos have been modified successfully. Ensure that you continue to use the todo list to track your progress. Please proceed with the current tasks if applicable".to_string(),
                error: None,
                data: Some(json!({
                    "workspace": workspace.display().to_string(),
                    "count": count,
                })),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tasks::models::{TaskGraph, TaskStatus as GraphTaskStatus};
    use chrono::Utc;

    /// CC 形态（camelCase activeForm）原样通过
    #[test]
    fn parses_cc_shaped_payload() {
        let params = json!({
            "todos": [
                { "content": "Run tests", "status": "in_progress", "activeForm": "Running tests" },
                { "content": "Build the project", "status": "pending", "activeForm": "Building the project" },
            ]
        });
        let parsed = parse_todo_write_params(&params).unwrap();
        assert_eq!(parsed.todos.len(), 2);
        assert_eq!(parsed.todos[0].status, TodoStatus::InProgress);
        assert_eq!(parsed.todos[0].active_form, "Running tests");
        assert_eq!(parsed.todos[1].status, TodoStatus::Pending);
    }

    /// snake_case `active_form` 同样接受
    #[test]
    fn accepts_snake_case_active_form() {
        let params = json!({
            "todos": [
                { "content": "安装 recharts 图表库", "status": "pending", "active_form": "安装 recharts 图表库中" },
            ]
        });
        let parsed = parse_todo_write_params(&params).unwrap();
        assert_eq!(parsed.todos[0].active_form, "安装 recharts 图表库中");
    }

    /// zod min(1) 对齐：content / activeForm 为空都报错
    #[test]
    fn rejects_empty_content_or_active_form() {
        for bad in [
            json!({ "todos": [{ "content": "  ", "status": "pending", "activeForm": "x" }] }),
            json!({ "todos": [{ "content": "x", "status": "pending", "activeForm": "" }] }),
        ] {
            assert!(parse_todo_write_params(&bad).is_err());
        }
    }

    /// strictObject 对齐：未知字段（包括旧 operation 包装）直接报错，而不是被宽容解析
    #[test]
    fn rejects_unknown_fields_and_legacy_envelope() {
        for bad in [
            json!({ "operation": { "action": "add", "title": "t" }, "todos": [] }),
            json!({ "todos": [{ "content": "t", "status": "pending", "activeForm": "t", "id": "1" }] }),
        ] {
            assert!(parse_todo_write_params(&bad).is_err());
        }
    }

    /// 状态枚举外的值（旧版的 Blocked 等）报错回给模型
    #[test]
    fn rejects_unknown_status() {
        let bad = json!({ "todos": [{ "content": "t", "status": "blocked", "activeForm": "t" }] });
        assert!(parse_todo_write_params(&bad).is_err());
    }

    /// 图构建：顺序保持、状态映射、active_form 落位
    #[test]
    fn next_todo_graph_maps_fields_and_order() {
        let params = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "completed", "activeForm": "doing a" },
                { "content": "b", "status": "in_progress", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let graph = next_todo_graph(&params.todos, &TaskGraph::new());
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.root_ids.len(), 2);
        let first = graph
            .nodes
            .get(&graph.root_ids[0])
            .expect("first node exists");
        assert_eq!(first.title, "a");
        assert_eq!(first.status, GraphTaskStatus::Completed);
        let second = graph
            .nodes
            .get(&graph.root_ids[1])
            .expect("second node exists");
        assert_eq!(second.status, GraphTaskStatus::InProgress);
        assert_eq!(second.active_form.as_deref(), Some("doing b"));
    }

    /// 全部 completed → 清单清空（CC：newTodos = allDone ? [] : todos）
    #[test]
    fn all_completed_clears_graph() {
        let params = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "completed", "activeForm": "doing a" },
                { "content": "b", "status": "completed", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let graph = next_todo_graph(&params.todos, &TaskGraph::new());
        assert!(graph.nodes.is_empty());
        assert!(graph.root_ids.is_empty());
    }

    /// 双重序列化的 content 不做任何解包 —— 就是普通字符串，原样展示。
    /// （纠偏：宽容兜底已在 86c5b85 引入、在本轮对标重构中移除。）
    #[test]
    fn double_encoded_content_stays_literal() {
        let raw = r#"{"title": "安装 recharts", "status": "in_progress"}"#;
        let params = parse_todo_write_params(&json!({
            "todos": [{ "content": raw, "status": "pending", "activeForm": "安装中" }]
        }))
        .unwrap();
        assert_eq!(params.todos[0].content, raw);
        let graph = next_todo_graph(&params.todos, &TaskGraph::new());
        let node = graph.nodes.get(&graph.root_ids[0]).expect("node exists");
        assert_eq!(node.title, raw);
    }

    /// 回归：completed 项必须带 `completed_at`，否则面板的 30s TTL
    /// （`TaskPanel::is_completed_expired`）永远判 false，✔ 项既不淡出也不
    /// 排到末尾，清单被已完成项顶满——用户看到的就是"Todo 有残留"。
    /// pending / in_progress 项则必须留 None，否则会被 TTL 误删。
    #[test]
    fn completed_items_get_timestamp_others_get_none() {
        let params = parse_todo_write_params(&json!({
            "todos": [
                { "content": "done", "status": "completed", "activeForm": "doing done" },
                { "content": "running", "status": "in_progress", "activeForm": "doing running" },
                { "content": "later", "status": "pending", "activeForm": "doing later" },
            ]
        }))
        .unwrap();
        let graph = next_todo_graph(&params.todos, &TaskGraph::new());
        let by_title = |t: &str| {
            graph
                .nodes
                .values()
                .find(|n| n.title == t)
                .unwrap_or_else(|| panic!("node {t} exists"))
        };
        assert!(
            by_title("done").completed_at.is_some(),
            "completed 必须有时间戳"
        );
        assert!(
            by_title("running").completed_at.is_none(),
            "in_progress 不能带旧时间戳"
        );
        assert!(
            by_title("later").completed_at.is_none(),
            "pending 不能带旧时间戳"
        );
    }

    /// 回归：TodoWrite 整表替换时节点 id 全换，`completed_at` 若一并盖成 now，
    /// 只要模型反复把已完成项原样发回来，它们就永不过期。命中同 content 的
    /// 旧节点必须沿用旧时间戳；新完成的才盖 now。
    #[test]
    fn completed_at_is_inherited_from_matching_previous_node() {
        let params = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "pending", "activeForm": "doing a" },
                { "content": "b", "status": "pending", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let mut prev = next_todo_graph(&params.todos, &TaskGraph::new());
        let old_stamp = Utc::now() - chrono::Duration::seconds(120);
        let node_a = prev
            .nodes
            .values_mut()
            .find(|n| n.title == "a")
            .expect("node a exists");
        node_a.status = GraphTaskStatus::Completed;
        node_a.completed_at = Some(old_stamp);

        // 模型把 a 原样标 completed 发回来，b 还没做
        let params2 = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "completed", "activeForm": "doing a" },
                { "content": "b", "status": "pending", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let graph = next_todo_graph(&params2.todos, &prev);
        let a = graph
            .nodes
            .values()
            .find(|n| n.title == "a")
            .expect("node a exists");
        assert_eq!(
            a.completed_at,
            Some(old_stamp),
            "同 content 的已完成项要沿用旧时间戳，否则 TTL 每次重写都重置"
        );
    }

    /// 第一次完成的项没有旧时间戳可沿用，盖当前时间。
    #[test]
    fn newly_completed_gets_fresh_timestamp() {
        let params = parse_todo_write_params(&json!({
            "todos": [{ "content": "a", "status": "pending", "activeForm": "doing a" }]
        }))
        .unwrap();
        let prev = next_todo_graph(&params.todos, &TaskGraph::new());
        let before = Utc::now();

        // b 保留 pending，否则全完成会触发"清单清空"，a 根本不进图
        let params2 = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "completed", "activeForm": "doing a" },
                { "content": "b", "status": "pending", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let graph = next_todo_graph(&params2.todos, &prev);
        let a = graph
            .nodes
            .values()
            .find(|n| n.title == "a")
            .expect("node a exists");
        let stamp = a.completed_at.expect("completed 项有时间戳");
        assert!(stamp >= before, "新完成项的时间戳不该早于写入时刻");
    }

    /// 从 completed 回到 pending/in_progress 要清掉 `completed_at`，
    /// 否则它仍会被 30s TTL 当成已完成项删掉。
    #[test]
    fn reopening_clears_completed_at() {
        let params = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "completed", "activeForm": "doing a" },
                { "content": "b", "status": "pending", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let prev = next_todo_graph(&params.todos, &TaskGraph::new());

        let params2 = parse_todo_write_params(&json!({
            "todos": [
                { "content": "a", "status": "in_progress", "activeForm": "doing a" },
                { "content": "b", "status": "pending", "activeForm": "doing b" },
            ]
        }))
        .unwrap();
        let graph = next_todo_graph(&params2.todos, &prev);
        let a = graph
            .nodes
            .values()
            .find(|n| n.title == "a")
            .expect("node a exists");
        assert_eq!(a.status, GraphTaskStatus::InProgress);
        assert!(a.completed_at.is_none(), "重开的项目必须清掉完成时间戳");
    }
}
