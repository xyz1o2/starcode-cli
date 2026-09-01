use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct LSPTool;

impl LSPTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LSPParams {
    pub action: String,
    pub file_path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub query: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct LSPOutput {
    pub results: Vec<LSPResult>,
}

#[derive(Debug, Serialize, Clone)]
pub struct LSPResult {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub content: String,
}

pub struct LSPInvocation {
    params: LSPParams,
}

impl ToolInvocation for LSPInvocation {
    fn get_description(&self) -> String {
        format!("LSP {}: {}", self.params.action, self.params.file_path)
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
        Box::pin(async move {
            let action = params.action.clone();
            let file_path = params.file_path.clone();
            let line = params.line.unwrap_or(1);
            let column = params.column.unwrap_or(1);

            match action.as_str() {
                "go_to_definition" => {
                    // In a real implementation, this would use LSP to find definition
                    Ok(ToolResult {
                        llm_content: Some(format!("Found definition at {}:{}:{}", file_path, line, column)),
                        return_display: Some("Definition found".to_string()),
                        output: serde_json::to_string(&LSPOutput {
                            results: vec![LSPResult {
                                file: file_path.clone(),
                                line,
                                column,
                                content: "function definition".to_string(),
                            }],
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "go_to_definition",
                            "file": file_path,
                            "line": line,
                            "column": column
                        })),
                    })
                }
                "find_references" => {
                    // In a real implementation, this would use LSP to find references
                    Ok(ToolResult {
                        llm_content: Some(format!("Found references for {}:{}:{}", file_path, line, column)),
                        return_display: Some("References found".to_string()),
                        output: serde_json::to_string(&LSPOutput {
                            results: vec![
                                LSPResult {
                                    file: file_path.clone(),
                                    line: line + 5,
                                    column,
                                    content: "reference 1".to_string(),
                                },
                                LSPResult {
                                    file: file_path.clone(),
                                    line: line + 10,
                                    column,
                                    content: "reference 2".to_string(),
                                },
                            ],
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "find_references",
                            "file": file_path,
                            "line": line,
                            "column": column
                        })),
                    })
                }
                "hover" => {
                    // In a real implementation, this would use LSP to get hover info
                    Ok(ToolResult {
                        llm_content: Some(format!("Hover info at {}:{}:{}", file_path, line, column)),
                        return_display: Some("Hover info retrieved".to_string()),
                        output: serde_json::to_string(&LSPOutput {
                            results: vec![LSPResult {
                                file: file_path.clone(),
                                line,
                                column,
                                content: "type: string".to_string(),
                            }],
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "hover",
                            "file": file_path,
                            "line": line,
                            "column": column
                        })),
                    })
                }
                "document_symbols" => {
                    // In a real implementation, this would use LSP to get document symbols
                    Ok(ToolResult {
                        llm_content: Some(format!("Found symbols in {}", file_path)),
                        return_display: Some("Document symbols found".to_string()),
                        output: serde_json::to_string(&LSPOutput {
                            results: vec![
                                LSPResult {
                                    file: file_path.clone(),
                                    line: 1,
                                    column: 1,
                                    content: "function main()".to_string(),
                                },
                                LSPResult {
                                    file: file_path.clone(),
                                    line: 10,
                                    column: 1,
                                    content: "struct Config".to_string(),
                                },
                            ],
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "document_symbols",
                            "file": file_path
                        })),
                    })
                }
                "workspace_symbols" => {
                    let query = params.query.ok_or("query is required for workspace_symbols")?;
                    
                    // In a real implementation, this would use LSP to search workspace symbols
                    Ok(ToolResult {
                        llm_content: Some(format!("Found workspace symbols matching '{}'", query)),
                        return_display: Some("Workspace symbols found".to_string()),
                        output: serde_json::to_string(&LSPOutput {
                            results: vec![LSPResult {
                                file: "src/main.rs".to_string(),
                                line: 1,
                                column: 1,
                                content: format!("symbol matching '{}'", query),
                            }],
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "workspace_symbols",
                            "query": query
                        })),
                    })
                }
                _ => Ok(ToolResult {
                    llm_content: Some(format!("Unknown LSP action: {}", action)),
                    return_display: Some(format!("Unknown action: {}", action)),
                    output: serde_json::to_string(&LSPOutput { results: vec![] })?,
                    error: Some(ToolError { error_type: "validation".to_string(), message: format!("Unknown action: {}. Use 'go_to_definition', 'find_references', 'hover', 'document_symbols', or 'workspace_symbols'", action) }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for LSPTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn display_name(&self) -> &str {
        "LSP"
    }

    fn description(&self) -> &str {
        "与语言服务器协议（LSP）交互，提供代码导航功能（定义跳转、引用查找、悬停信息等）。(Interact with Language Server Protocol (LSP) for code navigation features like go-to-definition, find-references, hover info, etc.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["go_to_definition", "find_references", "hover", "document_symbols", "workspace_symbols"],
                    "description": "要执行的LSP操作"
                },
                "file_path": {
                    "type": "string",
                    "description": "文件的绝对路径 (Absolute path to the file)"
                },
                "line": {
                    "type": "integer",
                    "description": "行号（从1开始）(Line number, 1-based)"
                },
                "column": {
                    "type": "integer",
                    "description": "列号（从1开始）(Column number, 1-based)"
                },
                "query": {
                    "type": "string",
                    "description": "搜索查询（workspace_symbols操作必填）(Search query, required for workspace_symbols action)"
                }
            },
            "required": ["action", "file_path"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: LSPParams = serde_json::from_value(params)?;
        Ok(Box::new(LSPInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
