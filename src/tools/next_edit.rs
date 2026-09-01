use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation,
    ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio::process::Command;

#[derive(Clone)]
pub struct NextEditTool;

impl NextEditTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NextEditParams {
    pub file_path: String,
    pub symbol_name: String,
    pub symbol_type: Option<String>, // "function", "struct", "enum", "method"
}

pub struct NextEditInvocation {
    params: NextEditParams,
}

impl ToolInvocation for NextEditInvocation {
    fn get_description(&self) -> String {
        format!(
            "Analyzing impact for symbol '{}' in {}",
            self.params.symbol_name, self.params.file_path
        )
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
        let file_path = self.params.file_path.clone();
        let symbol_name = self.params.symbol_name.clone();

        Box::pin(async move {
            let result = analyze_impact(&file_path, &symbol_name).await;

            match result {
                Ok(output) => Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output,
                    error: None,
                    data: None,
                }),
                Err(e) => Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "execution_error".to_string(),
                        message: e.to_string(),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for NextEditTool {
    fn name(&self) -> &str {
        "next_edit"
    }

    fn display_name(&self) -> &str {
        "Next Edit Analysis"
    }

    fn description(&self) -> &str {
        "Analyze the impact of changing a symbol (function, struct, etc.) by finding all references and dependencies."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file containing the symbol definition"
                },
                "symbol_name": {
                    "type": "string",
                    "description": "Name of the symbol (function, struct, etc.) to analyze"
                },
                "symbol_type": {
                    "type": "string",
                    "enum": ["function", "struct", "enum", "method"],
                    "description": "Optional type of the symbol to refine search"
                }
            },
            "required": ["file_path", "symbol_name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: NextEditParams = serde_json::from_value(params)?;
        Ok(Box::new(NextEditInvocation { params }))
    }
}

async fn analyze_impact(
    file_path: &str,
    symbol_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // 1. First, use ripgrep for fast text search to find potential usages
    // This is much faster than parsing every file
    let output = Command::new("rg")
        .arg("--json")
        .arg(symbol_name)
        .arg(".") // Search from current directory
        .output()
        .await?;

    if !output.status.success() {
        // If rg fails (e.g. no matches), return empty result
        return Ok(format!("No references found for '{}'.", symbol_name));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut usages: HashMap<String, Vec<u64>> = HashMap::new();
    let mut files_to_parse: HashSet<String> = HashSet::new();

    for line in stdout.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if json["type"] == "match" {
                if let Some(data) = json.get("data") {
                    if let Some(path) = data
                        .get("path")
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                    {
                        if let Some(line_num) = data.get("line_number").and_then(|l| l.as_u64()) {
                            usages.entry(path.to_string()).or_default().push(line_num);
                            if path.ends_with(".rs") {
                                files_to_parse.insert(path.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Report generation
    let mut report = String::new();
    report.push_str(&format!(
        "Impact Analysis for '{}' (defined in {}):\n\n",
        symbol_name, file_path
    ));

    let total_files = usages.len();
    let total_refs = usages.values().map(|v| v.len()).sum::<usize>();

    report.push_str(&format!(
        "Found {} references in {} files.\n\n",
        total_refs, total_files
    ));

    for (file, lines) in usages {
        report.push_str(&format!("File: {}\n", file));
        let lines_str: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        report.push_str(&format!("  Lines: {}\n", lines_str.join(", ")));

        // Context snippet for first match
        if let Some(first_line) = lines.first() {
            // Read file line
            if let Ok(content) = tokio::fs::read_to_string(&file).await {
                let content_lines: Vec<&str> = content.lines().collect();
                if let Some(line_content) =
                    content_lines.get((*first_line as usize).saturating_sub(1))
                {
                    report.push_str(&format!("  Snippet: {}\n", line_content.trim()));
                }
            }
        }
        report.push('\n');
    }

    Ok(report)
}
