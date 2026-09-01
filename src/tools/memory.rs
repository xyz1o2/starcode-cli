use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation,
    ToolResult as CoreToolResult,
};
use serde::Deserialize;
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct MemoryTool;

#[derive(Debug, Deserialize)]
pub struct MemoryParams {
    pub action: String,          // "save" | "read"
    pub content: Option<String>, // Required for save
    pub query: Option<String>,   // Optional for read, simple filter
}

pub struct MemoryToolInvocation {
    tool: MemoryTool,
    params: MemoryParams,
}

impl MemoryTool {
    pub fn new() -> Self {
        Self
    }

    async fn get_memory_file_path() -> Result<PathBuf, String> {
        let mut path = std::env::current_dir().map_err(|e| e.to_string())?;
        path.push(".star");
        if !path.exists() {
            fs::create_dir_all(&path).await.map_err(|e| e.to_string())?;
        }
        path.push("memory.md");
        Ok(path)
    }

    pub async fn execute_memory_op(
        &self,
        params: &MemoryParams,
    ) -> Result<CoreToolResult, Box<dyn std::error::Error>> {
        let path = Self::get_memory_file_path().await?;

        match params.action.as_str() {
            "save" => {
                let fact = params
                    .content
                    .as_ref()
                    .ok_or("Content required for save action")?;
                let mut current_content = if path.exists() {
                    fs::read_to_string(&path).await.unwrap_or_default()
                } else {
                    String::new()
                };

                if !current_content.is_empty() && !current_content.ends_with('\n') {
                    current_content.push('\n');
                }

                let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                let new_line = format!("- [{}] {}\n", now, fact);
                current_content.push_str(&new_line);

                fs::write(&path, current_content).await?;

                Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!("Memory saved to {}", path.display()),
                    error: None,
                    data: None,
                })
            }
            "read" => {
                if !path.exists() {
                    return Ok(CoreToolResult {
                        llm_content: None,
                        return_display: None,
                        output: "No memory file found yet.".to_string(),
                        error: None,
                        data: None,
                    });
                }
                let content = fs::read_to_string(&path).await?;
                let output = if let Some(q) = &params.query {
                    content
                        .lines()
                        .filter(|line| line.contains(q))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    content
                };

                Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: if output.is_empty() {
                        "No matching memories found.".to_string()
                    } else {
                        output
                    },
                    error: None,
                    data: None,
                })
            }
            _ => Ok(CoreToolResult {
                llm_content: None,
                return_display: None,
                output: String::new(),
                error: Some(ToolError {
                    error_type: "invalid_action".to_string(),
                    message: format!("Unknown action: {}. Use 'save' or 'read'.", params.action),
                }),
                data: None,
            }),
        }
    }
}

impl ToolInvocation for MemoryToolInvocation {
    fn get_description(&self) -> String {
        format!("Memory operation: {}", self.params.action)
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
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let tool = self.tool.clone();
        let params = MemoryParams {
            action: self.params.action.clone(),
            content: self.params.content.clone(),
            query: self.params.query.clone(),
        };

        Box::pin(async move { tool.execute_memory_op(&params).await })
    }
}

impl BaseDeclarativeTool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }

    fn display_name(&self) -> &str {
        "Memory Management"
    }

    fn description(&self) -> &str {
        "Manage long-term memory. Use 'save' to store facts, user preferences, or project decisions. Use 'read' to recall information."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "read"],
                    "description": "The action to perform."
                },
                "content": {
                    "type": "string",
                    "description": "The fact to save (required for 'save' action)."
                },
                "query": {
                    "type": "string",
                    "description": "Filter for reading memories (optional for 'read' action)."
                }
            },
            "required": ["action"]
        })
    }

    fn kind(&self) -> Kind {
        Kind::Execute // or Kind::Read/Write depending on action, but Execute is safe generic
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: MemoryParams = serde_json::from_value(params.clone())?;
        Ok(Box::new(MemoryToolInvocation {
            tool: self.clone(),
            params,
        }))
    }
}
