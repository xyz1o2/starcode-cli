use crate::core::tools::tools::{
    BaseDeclarativeTool, FunctionDeclaration, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::tools::ToolRegistry;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSearchParams {
    pub query: String,
}

pub struct ToolSearchTool {
    registry: Arc<ToolRegistry>,
}

impl ToolSearchTool {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    fn search_tools(&self, query: &str) -> Vec<FunctionDeclaration> {
        let query_lower = query.to_lowercase();
        let entries = self.registry.get_all_tool_entries();
        let mut results: Vec<FunctionDeclaration> = Vec::new();

        for (name, desc, parameters) in entries {
            let name_lower = name.to_lowercase();
            let desc_lower = desc.to_lowercase();

            if name_lower.contains(&query_lower) || desc_lower.contains(&query_lower) {
                results.push(FunctionDeclaration {
                    name,
                    description: desc,
                    parameters: parameters.clone(),
                    parameters_json_schema: parameters,
                });
            }
        }

        results
    }
}

impl BaseDeclarativeTool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn display_name(&self) -> &str {
        "Tool Search"
    }

    fn description(&self) -> &str {
        "Search for available tools (built-in and MCP) by name or description. \
         Use this to discover dynamically registered MCP tools before calling them."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search keywords to find matching tools by name or description"
                }
            },
            "required": ["query"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ToolSearchParams = serde_json::from_value(params)?;
        Ok(Box::new(ToolSearchInvocation {
            tool: self.clone(),
            params,
        }))
    }
}

impl Clone for ToolSearchTool {
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
        }
    }
}

pub struct ToolSearchInvocation {
    tool: ToolSearchTool,
    params: ToolSearchParams,
}

impl ToolInvocation for ToolSearchInvocation {
    fn get_description(&self) -> String {
        format!("Tool search: {}", self.params.query)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        Box::pin(async move { Ok(None) })
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
        let tool = self.tool.clone();
        let query = self.params.query.clone();

        Box::pin(async move {
            let results = tool.search_tools(&query);

            if results.is_empty() {
                return Ok(ToolResult {
                    llm_content: Some(format!(
                        "No tools found matching '{}'. Try different keywords.",
                        query
                    )),
                    return_display: None,
                    output: format!("No tools found for: {}", query),
                    error: None,
                    data: Some(serde_json::json!({
                        "matched_tools": [],
                        "total": 0,
                        "query": query
                    })),
                });
            }

            let mut lines = vec![format!(
                "Found {} tool(s) matching '{}':\n",
                results.len(),
                query
            )];

            for (i, tool) in results.iter().enumerate() {
                lines.push(format!(
                    "{}. **{}** — {}",
                    i + 1,
                    tool.name,
                    tool.description
                ));
            }

            let output = lines.join("\n");

            Ok(ToolResult {
                llm_content: Some(output.clone()),
                return_display: Some(format!(
                    "Found {} tool(s) matching '{}'",
                    results.len(),
                    query
                )),
                output,
                error: None,
                data: Some(serde_json::json!({
                    "matched_tools": results.iter().map(|t| serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                    })).collect::<Vec<_>>(),
                    "total": results.len(),
                    "query": query
                })),
            })
        })
    }
}

 