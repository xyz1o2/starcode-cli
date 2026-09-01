/// Editor Agent - 批量编辑专家
///
/// 职责：
/// - 批量代码编辑
/// - 重构操作
/// - 文件创建/删除
/// - 代码格式化
///
/// 使用的工具：
/// - smart_edit（智能编辑）
/// - create_file
/// - replace（降级）
use super::{SubAgent, SubTask, SubTaskResult};
use crate::agent::StarAgent;
use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::policy::PolicyEngine;
use crate::core::prompts::skills::editor::EDITOR_SYSTEM_PROMPT;
use crate::core::state::GlobalState;
use crate::core::tools::edit::{EditTool, EditToolParams};
use crate::core::tools::multi_edit::{MultiEditTool, MultiEditToolParams, SingleFileEdit};
use crate::core::tools::tools::{BaseDeclarativeTool, ToolResult};
use crate::core::tools::write_file::{WriteFileTool, WriteFileToolParams};
use crate::llm::client::StarClient;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::Arc;

pub struct EditorAgent {
    id: String,
    client: StarClient,
    config: Arc<Config>,
}

impl EditorAgent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        Self {
            id: "editor".to_string(),
            client,
            config,
        }
    }

    async fn try_fast_path(
        &self,
        task: &SubTask,
    ) -> Result<Option<SubTaskResult>, Box<dyn std::error::Error>> {
        let request = match resolve_fast_path_request(task, &self.config) {
            Some(request) => request,
            None => return Ok(None),
        };

        let message_bus = self
            .config
            .runtime_message_bus()
            .unwrap_or_else(|| Arc::new(MessageBus::new(PolicyEngine::default(), false)));
        let global_state = self
            .config
            .runtime_global_state()
            .unwrap_or_else(|| Arc::new(GlobalState::new()));

        let result = match request {
            EditorFastPathRequest::Single(params) => {
                let tool = EditTool::new(self.config.clone(), message_bus, global_state);
                let invocation = tool
                    .create_invocation(json!(params.clone()))
                    .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
                let tool_result = invocation.execute(None, None).await?;
                build_fast_path_result(
                    task,
                    "Edit",
                    json!({
                        "mode": "fast_path",
                        "tool": "Edit",
                        "file_path": params.file_path,
                    }),
                    tool_result,
                )
            }
            EditorFastPathRequest::Multi(params) => {
                let tool = MultiEditTool::new(self.config.clone(), global_state);
                let invocation = tool
                    .create_invocation(json!(params.clone()))
                    .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
                let tool_result = invocation.execute(None, None).await?;
                build_fast_path_result(
                    task,
                    "multi_edit",
                    json!({
                        "mode": "fast_path",
                        "tool": "multi_edit",
                        "files": params
                            .edits
                            .iter()
                            .map(|edit| edit.file_path.clone())
                            .collect::<Vec<_>>(),
                    }),
                    tool_result,
                )
            }
            EditorFastPathRequest::Write(params) => {
                let tool = WriteFileTool::new(self.config.clone(), message_bus, global_state);
                let invocation = tool
                    .create_invocation(json!(params.clone()))
                    .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
                let tool_result = invocation.execute(None, None).await?;
                build_fast_path_result(
                    task,
                    "Write",
                    json!({
                        "mode": "fast_path",
                        "tool": "Write",
                        "file_path": params.file_path,
                    }),
                    tool_result,
                )
            }
        };

        Ok(Some(result))
    }

    async fn run_edit_loop(
        &self,
        task: &SubTask,
    ) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        let mut agent = StarAgent::new(
            &self.client.api_key,
            Some(self.client.model.clone()),
            self.client.base_url.clone(),
            Some(10),
            Some(self.client.is_openai_compatible),
            Some(self.config.clone()),
        )
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        let prompt = format!(
            "{}\n\nTask: Perform edits based on objective.\nObjective: {}\nTarget: {}\nParams: {:?}",
            EDITOR_SYSTEM_PROMPT,
            task.objective,
            task.target,
            task.params
        );

        let entries = agent
            .process_user_message(&prompt)
            .await
            .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        let response = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == crate::types::ChatEntryType::Assistant)
            .map(|e| e.content.clone())
            .unwrap_or_else(|| "No response".to_string());

        Ok(
            SubTaskResult::success(task.id.clone(), "Editing Complete".to_string())
                .with_details(response),
        )
    }
}

fn build_fast_path_result(
    task: &SubTask,
    tool_name: &str,
    mut data: Value,
    tool_result: ToolResult,
) -> SubTaskResult {
    if let Some(tool_data) = tool_result.data.clone() {
        data["tool_data"] = tool_data;
    }

    let details = format_tool_result_details(&tool_result);

    if let Some(error) = tool_result.error {
        return SubTaskResult::failure(task.id.clone(), error.message)
            .with_details(details)
            .with_data(data);
    }

    SubTaskResult::success(task.id.clone(), "Editing Complete (fast path)".to_string())
        .with_details(if details.is_empty() {
            format!("Direct `{}` execution completed.", tool_name)
        } else {
            details
        })
        .with_data(data)
}

fn format_tool_result_details(tool_result: &ToolResult) -> String {
    let mut sections = Vec::new();

    if !tool_result.output.trim().is_empty() {
        sections.push(tool_result.output.trim().to_string());
    }

    if let Some(llm_content) = tool_result.llm_content.as_ref() {
        let llm_content = llm_content.trim();
        if !llm_content.is_empty() && llm_content != tool_result.output.trim() {
            sections.push(llm_content.to_string());
        }
    }

    if let Some(data) = tool_result.data.as_ref() {
        sections.push(format!(
            "Tool Data:\n{}",
            serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
        ));
    }

    sections.join("\n\n")
}

#[derive(Debug, Clone)]
enum EditorFastPathRequest {
    Single(EditToolParams),
    Multi(MultiEditToolParams),
    Write(WriteFileToolParams),
}

fn resolve_fast_path_request(
    task: &SubTask,
    config: &Arc<Config>,
) -> Option<EditorFastPathRequest> {
    if let Some(params) = resolve_multi_edit_params(task) {
        if params.edits.len() == 1 {
            let edit = params.edits.into_iter().next()?;
            return Some(EditorFastPathRequest::Single(EditToolParams {
                file_path: resolve_replace_path(config, &edit.file_path),
                old_string: edit.old_string,
                new_string: edit.new_string,
                expected_replacements: Some(1),
                instruction: None,
                modified_by_user: None,
                ai_proposed_content: None,
            }));
        }
        return Some(EditorFastPathRequest::Multi(params));
    }

    if let Some(params) = resolve_single_edit_params(task, config) {
        return Some(EditorFastPathRequest::Single(params));
    }

    resolve_write_file_params(task).map(EditorFastPathRequest::Write)
}

fn resolve_multi_edit_params(task: &SubTask) -> Option<MultiEditToolParams> {
    let edits_value = value_from_task(task, &["edits", "changes", "replacements"])?;
    let edits_array = edits_value.as_array()?;
    let fallback_path = if edits_array.len() == 1 {
        target_path_from_task(task)
    } else {
        None
    };

    let edits: Vec<SingleFileEdit> = edits_array
        .iter()
        .filter_map(|value| parse_single_file_edit(value, fallback_path.as_deref()))
        .collect();

    if edits.is_empty() || edits.len() != edits_array.len() {
        return None;
    }

    Some(MultiEditToolParams { edits })
}

fn resolve_single_edit_params(task: &SubTask, config: &Arc<Config>) -> Option<EditToolParams> {
    let file_path = task_string_param(task, &["file_path", "path", "file"])
        .or_else(|| target_path_from_task(task))?;
    let old_string = task_string_param(
        task,
        &["old_string", "old_str", "old", "oldString", "old_text"],
    )?;
    let new_string = task_string_param(
        task,
        &["new_string", "new_str", "new", "newString", "new_text"],
    )?;

    Some(EditToolParams {
        file_path: resolve_replace_path(config, &file_path),
        old_string,
        new_string,
        expected_replacements: task_usize_param(task, &["expected_replacements", "count"]),
        instruction: task_string_param(task, &["instruction", "note"]),
        modified_by_user: None,
        ai_proposed_content: None,
    })
}

fn resolve_write_file_params(task: &SubTask) -> Option<WriteFileToolParams> {
    let file_path = task_string_param(task, &["file_path", "path", "file"])
        .or_else(|| target_path_from_task(task))?;
    let content = task_string_param(task, &["content", "new_content", "body", "text"])?;

    Some(WriteFileToolParams {
        file_path,
        content,
        modified_by_user: None,
        ai_proposed_content: None,
    })
}

fn resolve_replace_path(config: &Arc<Config>, file_path: &str) -> String {
    crate::core::utils::paths::resolve_tool_path(config.target_dir(), file_path)
        .to_string_lossy()
        .to_string()
}

fn parse_single_file_edit(value: &Value, fallback_path: Option<&str>) -> Option<SingleFileEdit> {
    let object = value.as_object()?;
    let file_path = string_from_object(object, &["file_path", "path", "file", "target"])
        .or_else(|| fallback_path.map(|path| path.to_string()))?;
    let old_string = string_from_object(
        object,
        &["old_string", "old_str", "old", "oldString", "old_text"],
    )?;
    let new_string = string_from_object(
        object,
        &["new_string", "new_str", "new", "newString", "new_text"],
    )?;

    Some(SingleFileEdit {
        file_path,
        old_string,
        new_string,
    })
}

fn value_from_task<'a>(task: &'a SubTask, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| task.params.get(*key))
}

fn task_string_param(task: &SubTask, keys: &[&str]) -> Option<String> {
    value_from_task(task, keys).and_then(value_to_string)
}

fn task_usize_param(task: &SubTask, keys: &[&str]) -> Option<usize> {
    value_from_task(task, keys).and_then(|value| match value {
        Value::Number(number) => number.as_u64().map(|n| n as usize),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    })
}

fn target_path_from_task(task: &SubTask) -> Option<String> {
    let target = task.target.trim();
    if target.is_empty() || target == "." {
        None
    } else {
        Some(target.to_string())
    }
}

fn string_from_object(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(value_to_string))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[async_trait]
impl SubAgent for EditorAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Editor Agent (代码编辑专家)"
    }

    fn capabilities(&self) -> Vec<String> {
        vec![
            "edit".to_string(),
            "Edit".to_string(),
            "refactor".to_string(),
            "modify".to_string(),
        ]
    }

    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        if let Some(result) = self.try_fast_path(&task).await? {
            return Ok(result);
        }

        self.run_edit_loop(&task).await
    }
}
