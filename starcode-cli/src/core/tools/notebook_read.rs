use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookReadParams {
    pub file_path: String,
    pub include_outputs: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NotebookCell {
    cell_type: String,
    execution_count: Option<i32>,
    metadata: Value,
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

pub struct NotebookReadInvocation {
    config: Arc<crate::core::config::Config>,
    params: NotebookReadParams,
}

impl NotebookReadInvocation {
    pub fn new(config: Arc<crate::core::config::Config>, params: NotebookReadParams) -> Self {
        Self { config, params }
    }
}

impl ToolInvocation for NotebookReadInvocation {
    fn get_description(&self) -> String {
        format!("Read notebook {}", self.params.file_path)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        let path = self.config.target_dir().join(&self.params.file_path);
        vec![ToolLocation {
            path,
            location_type: crate::core::tools::tools::LocationType::Read,
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
        // Clone self to call format_notebook method, or move logic to associated function.
        // To avoid lifetime issues with 'self' in async block, we'll just copy the format logic or move it.
        // Actually, we can't easily call self.format_notebook inside the async block if we move self.params.
        // Let's make format_notebook a standalone function or method on params/struct that doesn't capture self.

        let include_outputs = params.include_outputs;
        let file_path_str = params.file_path.clone();

        Box::pin(async move {
            let path = config.target_dir().join(&file_path_str);

            if !path.exists() {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "file_not_found".to_string(),
                        message: format!("File not found: {}", file_path_str),
                    }),
                    data: None,
                });
            }

            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| e.to_string())?;

            // Logic duplicated here to avoid self capture issues
            let notebook: Notebook = serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse notebook JSON: {}", e))?;

            let mut output = String::new();
            output.push_str(&format!("Notebook: {}\n", file_path_str));
            output.push_str(&format!(
                "Format: v{}.{}\n",
                notebook.nbformat, notebook.nbformat_minor
            ));
            output.push_str(&format!("Total Cells: {}\n", notebook.cells.len()));
            output.push_str("--------------------------------------------------\n");

            for (i, cell) in notebook.cells.iter().enumerate() {
                output.push_str(&format!("Cell {} [{}]:\n", i, cell.cell_type));

                let source = cell.source.join("");
                output.push_str("```\n");
                output.push_str(&source);
                if !source.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("```\n");

                if include_outputs.unwrap_or(true) {
                    if let Some(outputs) = &cell.outputs {
                        if !outputs.is_empty() {
                            output.push_str("Outputs:\n");
                            for out in outputs {
                                // Simple output extraction
                                if let Some(text) = out.get("text") {
                                    if let Some(lines) = text.as_array() {
                                        for line in lines {
                                            if let Some(s) = line.as_str() {
                                                output.push_str(s);
                                            }
                                        }
                                    } else if let Some(s) = text.as_str() {
                                        output.push_str(s);
                                    }
                                } else if let Some(data) = out.get("data") {
                                    if let Some(text_plain) = data.get("text/plain") {
                                        if let Some(lines) = text_plain.as_array() {
                                            for line in lines {
                                                if let Some(s) = line.as_str() {
                                                    output.push_str(s);
                                                }
                                            }
                                        } else if let Some(s) = text_plain.as_str() {
                                            output.push_str(s);
                                        }
                                    }
                                } else if let Some(output_type) = out.get("output_type") {
                                    if output_type == "error" {
                                        if let Some(ename) =
                                            out.get("ename").and_then(|v| v.as_str())
                                        {
                                            output.push_str(&format!("Error: {}\n", ename));
                                        }
                                        if let Some(evalue) =
                                            out.get("evalue").and_then(|v| v.as_str())
                                        {
                                            output.push_str(&format!("Value: {}\n", evalue));
                                        }
                                    }
                                }
                                output.push('\n');
                            }
                        }
                    }
                }
                output.push_str("--------------------------------------------------\n");
            }

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(output.clone()),
                output,
                error: None,
                data: None,
            })
        })
    }
}

pub struct NotebookReadTool {
    config: Arc<crate::core::config::Config>,
}

impl NotebookReadTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

impl BaseDeclarativeTool for NotebookReadTool {
    fn name(&self) -> &str {
        "notebook_read"
    }

    fn display_name(&self) -> &str {
        "Notebook Read"
    }

    fn description(&self) -> &str {
        "Reads a Jupyter Notebook (.ipynb) file and returns a formatted text representation of its cells and outputs. Useful for understanding code and results in notebooks."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the .ipynb file"
                },
                "include_outputs": {
                    "type": "boolean",
                    "description": "Whether to include cell outputs in the response (default: true)"
                }
            },
            "required": ["file_path"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: NotebookReadParams = serde_json::from_value(params)?;
        Ok(Box::new(NotebookReadInvocation::new(
            self.config.clone(),
            params,
        )))
    }
}
