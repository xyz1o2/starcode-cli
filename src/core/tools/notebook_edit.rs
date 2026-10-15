use crate::core::state::GlobalState;
use crate::core::tools::constants::ToolErrorType;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookEditParams {
    pub file_path: String,
    pub edit_type: EditType,
    pub cell_index: usize,
    pub content: Option<String>,
    pub cell_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EditType {
    UpdateCell,
    InsertCell,
    DeleteCell,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct NotebookCell {
    cell_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    execution_count: Option<i32>,
    metadata: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    outputs: Option<Vec<Value>>,
    source: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Notebook {
    cells: Vec<NotebookCell>,
    metadata: Value,
    nbformat: i32,
    nbformat_minor: i32,
}

pub struct NotebookEditInvocation {
    config: Arc<crate::core::config::Config>,
    params: NotebookEditParams,
    global_state: Arc<GlobalState>,
}

impl NotebookEditInvocation {
    pub fn new(
        config: Arc<crate::core::config::Config>,
        params: NotebookEditParams,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
        global_state: Arc<GlobalState>,
    ) -> Self {
        Self {
            config,
            params,
            global_state,
        }
    }
}

impl ToolInvocation for NotebookEditInvocation {
    fn get_description(&self) -> String {
        format!(
            "Edit notebook {}: {:?}",
            self.params.file_path, self.params.edit_type
        )
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        let path = self.config.target_dir().join(&self.params.file_path);
        vec![ToolLocation {
            path,
            location_type: crate::core::tools::tools::LocationType::Write,
        }]
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
        let config = self.config.clone();
        let params = self.params.clone();
        let global_state = self.global_state.clone();

        Box::pin(async move {
            let mut _file_contents: HashMap<PathBuf, String> = HashMap::new();
            let mut _final_contents: HashMap<PathBuf, String> = HashMap::new();

            let path = config.target_dir().join(&params.file_path);

            if !path.exists() {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "file_not_found".to_string(),
                        message: format!("File not found: {}", params.file_path),
                    }),
                    data: None,
                });
            }

            // ============ P1.2 改进：前置验证 - 检查 notebook 是否已被读取 ============
            let strict_read_check = std::env::var("STAR_DISABLE_READ_CHECK")
                .map(|v| v.to_lowercase() != "true" && v != "1")
                .unwrap_or(true);

            if strict_read_check {
                // 键必须和 star_agent::mark_file_as_read 写表时用同一个函数算
                // （见 paths::read_state_key），否则读过的 notebook 仍会被拦下。
                let abs_path = crate::core::utils::paths::read_state_key(&path);

                {
                    // 查 `read_file_state`（而不是 `execution_state`）：`notebook_read`
                    // 写的是前者，后者只有单个 `Read` 和 `@` 内联会写。查错表的结果
                    // 是 `notebook_read` → `notebook_edit` 这条被工具说明明确要求的
                    // 路径 100% 被拦下。
                    let read_state = global_state.read_file_state.read().await;
                    if !read_state.contains_key(&abs_path) {
                        let msg = format!(
                            "Edit blocked [edit_file_not_read]: notebook '{}' must be read with `notebook_read` before using `notebook_edit`. \
                             REQUIRED NEXT STEP: call `notebook_read` with file_path='{}' first, then retry `notebook_edit`. \
                             Do NOT retry without reading the notebook first.",
                            params.file_path, params.file_path
                        );
                        return Ok(ToolResult {
                            llm_content: Some(msg.clone()),
                            return_display: Some(msg.clone()),
                            output: msg.clone(),
                            error: Some(ToolError {
                                error_type: ToolErrorType::EditFileNotRead.to_string(),
                                message: msg,
                            }),
                            data: None,
                        });
                    }
                } // 显式释放读锁
            }

            // Read
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| e.to_string())?;
            // 和 `notebook_read` 对齐：解析失败必须走 `ToolResult.error`，不能裸
            // `Err` 出去。裸 `Err` 在执行器里拿不到文件名和解析器的列号，模型只看到
            // 一句没有上下文的字符串，只能反复重试同一个坏文件。
            let mut notebook: Notebook = match serde_json::from_str(&content) {
                Ok(notebook) => notebook,
                Err(error) => {
                    let message = format!(
                        "Failed to parse notebook JSON in '{}': {}",
                        path.display(),
                        error
                    );
                    return Ok(ToolResult {
                        llm_content: Some(message.clone()),
                        return_display: Some(message.clone()),
                        output: message.clone(),
                        error: Some(ToolError {
                            error_type: "invalid_notebook_json".to_string(),
                            message,
                        }),
                        data: None,
                    });
                }
            };

            // Modify
            let message;

            match params.edit_type {
                EditType::UpdateCell => {
                    if params.cell_index >= notebook.cells.len() {
                        return Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: String::new(),
                            error: Some(ToolError {
                                error_type: "index_error".to_string(),
                                message: format!(
                                    "Cell index {} out of range (total {})",
                                    params.cell_index,
                                    notebook.cells.len()
                                ),
                            }),
                            data: None,
                        });
                    }

                    if let Some(new_content) = params.content {
                        // Split content into lines, keeping newlines
                        let lines: Vec<String> = new_content
                            .split_inclusive('\n')
                            .map(|s| s.to_string())
                            .collect();
                        notebook.cells[params.cell_index].source = lines;
                    }

                    if let Some(new_type) = params.cell_type {
                        notebook.cells[params.cell_index].cell_type = new_type;
                    }

                    message = format!("Updated cell {} in {}", params.cell_index, params.file_path);
                }
                EditType::DeleteCell => {
                    if params.cell_index >= notebook.cells.len() {
                        return Ok(ToolResult {
                            llm_content: None,
                            return_display: None,
                            output: String::new(),
                            error: Some(ToolError {
                                error_type: "index_error".to_string(),
                                message: format!(
                                    "Cell index {} out of range (total {})",
                                    params.cell_index,
                                    notebook.cells.len()
                                ),
                            }),
                            data: None,
                        });
                    }
                    notebook.cells.remove(params.cell_index);
                    message = format!(
                        "Deleted cell {} from {}",
                        params.cell_index, params.file_path
                    );
                }
                EditType::InsertCell => {
                    let cell_type = params
                        .cell_type
                        .clone()
                        .unwrap_or_else(|| "code".to_string());
                    let source_content = params.content.unwrap_or_default();
                    let lines: Vec<String> = source_content
                        .split_inclusive('\n')
                        .map(|s| s.to_string())
                        .collect();

                    let new_cell = NotebookCell {
                        cell_type: cell_type.clone(),
                        execution_count: None,
                        metadata: json!({}),
                        outputs: if cell_type == "code" {
                            Some(vec![])
                        } else {
                            None
                        },
                        source: lines,
                    };

                    if params.cell_index > notebook.cells.len() {
                        notebook.cells.push(new_cell);
                    } else {
                        notebook.cells.insert(params.cell_index, new_cell);
                    }
                    message = format!(
                        "Inserted new cell at index {} in {}",
                        params.cell_index, params.file_path
                    );
                }
            }

            // Write
            let new_json = serde_json::to_string_pretty(&notebook)
                .map_err(|e| format!("Failed to serialize notebook: {}", e))?;

            tokio::fs::write(&path, new_json)
                .await
                .map_err(|e| e.to_string())?;

            Ok(ToolResult {
                llm_content: Some(message.clone()),
                return_display: Some(message.clone()),
                output: message,
                error: None,
                data: None,
            })
        })
    }
}

pub struct NotebookEditTool {
    config: Arc<crate::core::config::Config>,
    global_state: Arc<GlobalState>,
}

impl NotebookEditTool {
    pub fn new(config: Arc<crate::core::config::Config>, global_state: Arc<GlobalState>) -> Self {
        Self {
            config,
            global_state,
        }
    }
}

impl BaseDeclarativeTool for NotebookEditTool {
    fn name(&self) -> &str {
        "notebook_edit"
    }

    fn display_name(&self) -> &str {
        "Notebook Edit"
    }

    fn description(&self) -> &str {
        "Edits a Jupyter Notebook (.ipynb) file. Supports updating, inserting, and deleting cells."
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the .ipynb file"
                },
                "edit_type": {
                    "type": "string",
                    "enum": ["update_cell", "insert_cell", "delete_cell"],
                    "description": "Type of edit operation"
                },
                "cell_index": {
                    "type": "integer",
                    "description": "Index of the cell to edit/delete/insert at"
                },
                "content": {
                    "type": "string",
                    "description": "New content for the cell (for update/insert)"
                },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown"],
                    "description": "Type of cell (for update/insert)"
                }
            },
            "required": ["file_path", "edit_type", "cell_index"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: NotebookEditParams = serde_json::from_value(params)?;
        Ok(Box::new(NotebookEditInvocation::new(
            self.config.clone(),
            params,
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
            self.global_state.clone(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn notebook_json() -> String {
        json!({
            "cells": [
                { "cell_type": "code", "execution_count": null, "metadata": {},
                  "outputs": [], "source": ["print('one')"] },
                { "cell_type": "markdown", "metadata": {},
                  "source": ["# heading"] }
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        })
        .to_string()
    }

    async fn harness() -> (
        tempfile::TempDir,
        Arc<crate::core::config::Config>,
        Arc<crate::core::state::GlobalState>,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut params = crate::core::config::ConfigParameters::default();
        params.model = "test-model".to_string();
        params.core_tools = Some(Vec::new());
        params.mcp_enabled = Some(false);
        params.enable_agents = Some(false);
        // 工具用 `config.target_dir()` 解析相对路径，不是 `cwd`。
        params.target_dir = dir.path().to_path_buf();
        let mut config = crate::core::config::Config::new(params);
        config
            .initialize()
            .await
            .expect("test config must initialize");
        // 这个断言是硬防线：`notebook_edit` 会真的往磁盘写。`target_dir` 一旦没指对
        // （比如只设了 `cwd`），测试就会把改动写进**用户的真实仓库**而不是临时目录，
        // 而且不会有任何报错。设错时的实际后果：仓库根目录多出一个 `nb.ipynb`。
        assert_eq!(
            config.target_dir(),
            dir.path(),
            "harness 必须把 target_dir 指到临时目录，否则测试会写进真实仓库"
        );
        (
            dir,
            Arc::new(config),
            Arc::new(crate::core::state::GlobalState::new()),
        )
    }

    async fn run(
        config: &Arc<crate::core::config::Config>,
        gs: &Arc<GlobalState>,
        params: serde_json::Value,
    ) -> ToolResult {
        NotebookEditTool::new(config.clone(), gs.clone())
            .create_invocation(params)
            .expect("create_invocation should accept these params")
            .execute(None, None)
            .await
            .expect("execute should not return a bare Err")
    }

    async fn mark_read(gs: &Arc<GlobalState>, path: &std::path::Path, content: &str) {
        let key = crate::core::utils::paths::read_state_key(path);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        crate::core::tools::read_file::record_file_read(gs, &key, content.to_string(), now, now)
            .await;
    }

    /// 判别式回归：门必须查 `read_file_state`。
    ///
    /// `notebook_read` 只写那张表；查 `execution_state` 的话
    /// "notebook_read → notebook_edit" 这条被工具说明写死的路径 100% 被拦下。
    #[tokio::test]
    async fn a_read_notebook_can_then_be_edited() {
        let (dir, config, gs) = harness().await;
        let nb = dir.path().join("nb.ipynb");
        std::fs::write(&nb, notebook_json()).unwrap();
        mark_read(&gs, &nb, &notebook_json()).await;

        let result = run(
            &config,
            &gs,
            json!({
                "file_path": "nb.ipynb",
                "edit_type": "update_cell",
                "cell_index": 0,
                "content": "print('two')"
            }),
        )
        .await;

        assert!(
            result.error.is_none(),
            "读过的 notebook 必须放行：{:?}",
            result.error
        );
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&nb).unwrap()).unwrap();
        assert_eq!(
            after["cells"][0]["source"][0], "print('two')",
            "cell 内容必须真的被改到磁盘"
        );
    }

    /// 没读过的 notebook 必须被拦下，且错误可执行。
    #[tokio::test]
    async fn an_unread_notebook_is_blocked_with_a_read_instruction() {
        let (dir, config, gs) = harness().await;
        let nb = dir.path().join("nb.ipynb");
        std::fs::write(&nb, notebook_json()).unwrap();

        let result = run(
            &config,
            &gs,
            json!({
                "file_path": "nb.ipynb",
                "edit_type": "delete_cell",
                "cell_index": 0
            }),
        )
        .await;

        let error = result.error.expect("未读取必须报错");
        assert_eq!(
            error.error_type,
            crate::core::tools::constants::EDIT_FILE_NOT_READ_MARKER,
            "error_type 必须是 agent 识别的标记"
        );
        assert!(
            error.message.contains("notebook_read"),
            "必须点名 notebook_read：{:?}",
            error.message
        );
        assert_eq!(
            std::fs::read_to_string(&nb).unwrap(),
            notebook_json(),
            "被拦下时磁盘不能被改"
        );
    }

    /// 越界的 cell_index 必须报错，不能 panic（`notebook.cells[i]` 是直接索引）。
    #[tokio::test]
    async fn an_out_of_range_cell_index_errors_instead_of_panicking() {
        let (dir, config, gs) = harness().await;
        let nb = dir.path().join("nb.ipynb");
        std::fs::write(&nb, notebook_json()).unwrap();
        mark_read(&gs, &nb, &notebook_json()).await;

        let result = run(
            &config,
            &gs,
            json!({ "file_path": "nb.ipynb", "edit_type": "delete_cell", "cell_index": 99 }),
        )
        .await;

        let error = result.error.expect("越界必须报错");
        assert!(
            error.message.contains("out of range"),
            "错误要说清是越界：{:?}",
            error.message
        );
    }

    /// 插入 cell 后 notebook 仍是合法 JSON，且 cell 数 +1。
    #[tokio::test]
    async fn inserting_a_cell_keeps_the_notebook_valid_and_grows_it() {
        let (dir, config, gs) = harness().await;
        let nb = dir.path().join("nb.ipynb");
        std::fs::write(&nb, notebook_json()).unwrap();
        mark_read(&gs, &nb, &notebook_json()).await;

        let result = run(
            &config,
            &gs,
            json!({
                "file_path": "nb.ipynb",
                "edit_type": "insert_cell",
                "cell_index": 1,
                "content": "new cell",
                "cell_type": "code"
            }),
        )
        .await;

        assert!(result.error.is_none(), "不该报错：{:?}", result.error);
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&nb).unwrap()).unwrap();
        assert_eq!(after["cells"].as_array().unwrap().len(), 3);
        assert_eq!(after["cells"][1]["source"][0], "new cell");
    }

    /// 缺 required 参数在 `create_invocation` 就被拒。
    #[tokio::test]
    async fn missing_required_params_are_rejected_at_creation() {
        let (_dir, config, gs) = harness().await;
        assert!(
            NotebookEditTool::new(config, gs)
                .create_invocation(json!({ "file_path": "nb.ipynb" }))
                .is_err(),
            "缺 edit_type/cell_index 必须在建 invocation 时报错"
        );
    }
}
