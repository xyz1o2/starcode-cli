//! Tool Search 工具

use async_trait::async_trait;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolMeta {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub kind: String,
    pub category: String,
    pub keywords: Vec<String>,
}

pub struct ToolSearchIndex {
    tools: Vec<ToolMeta>,
}

impl ToolSearchIndex {
    pub fn new() -> Self { Self { tools: Vec::new() } }

    pub fn register(&mut self, meta: ToolMeta) { self.tools.push(meta); }

    pub fn search(&self, query: &str, limit: usize) -> Vec<ToolMeta> {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(f64, &ToolMeta)> = self.tools.iter()
            .filter_map(|tool| {
                let score = self.calculate_score(tool, &query_lower, &query_words);
                if score > 0.0 { Some((score, tool)) } else { None }
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(limit).map(|(_, tool)| tool.clone()).collect()
    }

    fn calculate_score(&self, tool: &ToolMeta, query: &str, query_words: &[&str]) -> f64 {
        let mut score = 0.0;
        let name_lower = tool.name.to_lowercase();
        let display_lower = tool.display_name.to_lowercase();
        let desc_lower = tool.description.to_lowercase();

        if name_lower == *query || display_lower == *query { score += 100.0; }
        if name_lower.starts_with(query) || display_lower.starts_with(query) { score += 50.0; }
        if name_lower.contains(query) || display_lower.contains(query) { score += 30.0; }

        for word in query_words {
            if name_lower.contains(word) { score += 20.0; }
            if display_lower.contains(word) { score += 15.0; }
            if desc_lower.contains(word) { score += 5.0; }
        }
        score
    }

    pub fn list_all(&self) -> &[ToolMeta] { &self.tools }
}

pub struct ToolSearchTool {
    index: Arc<std::sync::Mutex<ToolSearchIndex>>,
}

impl ToolSearchTool {
    pub fn new(index: Arc<std::sync::Mutex<ToolSearchIndex>>) -> Self { Self { index } }
}

pub struct ToolSearchInvocation {
    query: String,
    limit: usize,
    action: String,
    index: Arc<std::sync::Mutex<ToolSearchIndex>>,
}

#[async_trait]
impl ToolInvocation for ToolSearchInvocation {
    fn get_description(&self) -> String { format!("Tool search: {}", self.query) }
    fn tool_locations(&self) -> Vec<ToolLocation> { Vec::new() }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, Box<dyn std::error::Error>>> + Send + '_>>
    {
        let query = self.query.clone();
        let limit = self.limit;
        let action = self.action.clone();
        let index = self.index.clone();

        Box::pin(async move {
            let idx = index.lock().unwrap();

            match action.as_str() {
                "list" => {
                    let all = idx.list_all();
                    Ok(ToolResult {
                        llm_content: Some(format!("{} tools available", all.len())),
                        return_display: Some(format!("{} tools available", all.len())),
                        output: String::new(),
                        error: None,
                        data: Some(json!({ "tools": all })),
                    })
                }
                _ => {
                    if query.is_empty() {
                        return Ok(ToolResult {
                            output: "Search query cannot be empty".to_string(),
                            error: Some(ToolError { error_type: "validation".to_string(), message: "Empty query".to_string() }),
                            ..Default::default()
                        });
                    }

                    let results = idx.search(&query, limit);
                    if results.is_empty() {
                        return Ok(ToolResult {
                            llm_content: Some(format!("No tools found matching '{}'", query)),
                            return_display: Some(format!("No tools found matching '{}'", query)),
                            output: String::new(),
                            error: None,
                            data: Some(json!({ "results": [], "query": query })),
                        });
                    }

                    let output = results.iter()
                        .map(|t| format!("- {} ({})", t.name, t.description))
                        .collect::<Vec<_>>()
                        .join("\n");

                    Ok(ToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(output),
                        output: String::new(),
                        error: None,
                        data: Some(json!({ "results": results, "query": query })),
                    })
                }
            }
        })
    }
}

impl BaseDeclarativeTool for ToolSearchTool {
    fn name(&self) -> &str { "tool_search" }
    fn display_name(&self) -> &str { "Tool Search" }
    fn description(&self) -> &str { "Search and discover available tools." }
    fn kind(&self) -> Kind { Kind::Read }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query" },
                "action": { "type": "string", "enum": ["search", "list"], "description": "Action (default: search)" },
                "limit": { "type": "integer", "description": "Max results (default: 10)", "minimum": 1, "maximum": 50 }
            },
            "required": ["query"]
        })
    }

    fn create_invocation(&self, params: Value) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("").to_string();
        let limit = params.get("limit").and_then(|l| l.as_u64()).map(|l| l as usize).unwrap_or(10);
        let action = params.get("action").and_then(|a| a.as_str()).unwrap_or("search").to_string();

        Ok(Box::new(ToolSearchInvocation { query, limit, action, index: self.index.clone() }))
    }
}

pub fn build_default_tool_index() -> ToolSearchIndex {
    let mut index = ToolSearchIndex::new();

    let tools = vec![
        ("Read", "Read File", "Read the contents of a file", "Read", "file"),
        ("Edit", "Edit File", "Edit a file by replacing exact strings", "Edit", "file"),
        ("Write", "Write File", "Write content to a file", "Edit", "file"),
        ("Bash", "Shell Command", "Execute shell commands", "Execute", "shell"),
        ("Grep", "Search Content", "Search file contents using regex", "Search", "search"),
        ("Glob", "Find Files", "Find files by glob pattern", "Search", "search"),
        ("todo", "Task Management", "Create, update, and manage tasks", "Think", "task"),
        ("memory", "Memory", "Store and retrieve persistent memories", "Think", "memory"),
        ("web_search", "Web Search", "Search the web for information", "Read", "web"),
        ("enter_plan_mode", "Enter Plan Mode", "Enter read-only plan mode", "Think", "plan"),
        ("exit_plan_mode", "Exit Plan Mode", "Exit plan mode and submit plan", "Think", "plan"),
    ];

    for (name, display, desc, kind, cat) in tools {
        index.register(ToolMeta {
            name: name.into(),
            display_name: display.into(),
            description: desc.into(),
            kind: kind.into(),
            category: cat.into(),
            keywords: vec![name.to_lowercase()],
        });
    }

    index
}
