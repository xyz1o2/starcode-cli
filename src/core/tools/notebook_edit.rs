use crate::core::state::GlobalState;
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
                let abs_path = path
                    .canonicalize()
                    .unwrap_or_else(|_| path.clone())
                    .to_string_lossy()
                    .to_string();

                {
                    let exec_state = global_state.execution_state.read().await;
                    if !exec_state.was_file_read(&abs_path) {
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
                                error_type: "file_has_not_been_read".to_string(),
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
            let mut notebook: Notebook = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse notebook JSON: {}", e))?;

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
