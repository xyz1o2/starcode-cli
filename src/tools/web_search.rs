//! Web Search 工具

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
}

pub struct WebSearchTool {
    default_results: usize,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self { default_results: 5 }
    }

    async fn search_ddg(
        &self,
        query: &str,
        num_results: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let client = reqwest::Client::new();
        let response = client
            .get("https://api.duckduckgo.com/")
            .query(&[
                ("q", query),
                ("format", &"json".to_string()),
                ("no_redirect", &"1".to_string()),
            ])
            .send()
            .await
            .map_err(|e| format!("DuckDuckGo API error: {}", e))?;

        let resp: Value = response
            .json()
            .await
            .map_err(|e| format!("JSON error: {}", e))?;
        let mut results = Vec::new();

        if let Some(abstract_text) = resp["Abstract"].as_str() {
            if !abstract_text.is_empty() {
                results.push(SearchResult {
                    title: resp["Heading"].as_str().unwrap_or("").to_string(),
                    url: resp["AbstractURL"].as_str().unwrap_or("").to_string(),
                    snippet: abstract_text.to_string(),
                    source: "duckduckgo".to_string(),
                });
            }
        }

        if let Some(topics) = resp["RelatedTopics"].as_array() {
            for topic in topics
                .iter()
                .take(num_results.saturating_sub(results.len()))
            {
                if let Some(text) = topic["Text"].as_str() {
                    if !text.is_empty() {
                        results.push(SearchResult {
                            title: text.chars().take(80).collect(),
                            url: topic["FirstURL"].as_str().unwrap_or("").to_string(),
                            snippet: text.to_string(),
                            source: "duckduckgo".to_string(),
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}

pub struct WebSearchInvocation {
    query: String,
    num_results: usize,
    tool: WebSearchTool,
}

#[async_trait]
impl ToolInvocation for WebSearchInvocation {
    fn get_description(&self) -> String {
        format!("Web search: {}", self.query)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        Vec::new()
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, Box<dyn std::error::Error>>> + Send + '_>>
    {
        let query = self.query.clone();
        let num_results = self.num_results;

        Box::pin(async move {
            if query.is_empty() {
                return Ok(ToolResult {
                    output: "Search query cannot be empty".to_string(),
                    error: Some(ToolError {
                        error_type: "validation".to_string(),
                        message: "Empty query".to_string(),
                    }),
                    ..Default::default()
                });
            }

            match self.tool.search_ddg(&query, num_results).await {
                Ok(results) if !results.is_empty() => {
                    let output = results
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            format!("{}. {}\n   {}\n   {}\n", i + 1, r.title, r.url, r.snippet)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    Ok(ToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(output),
                        output: String::new(),
                        error: None,
                        data: Some(json!({ "query": query, "results": results })),
                    })
                }
                Ok(_) => Ok(ToolResult {
                    llm_content: Some(format!("No results found for '{}'", query)),
                    return_display: Some(format!("No results found for '{}'", query)),
                    output: String::new(),
                    error: None,
                    data: Some(json!({ "query": query, "results": [] })),
                }),
                Err(e) => Ok(ToolResult {
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "search".to_string(),
                        message: e,
                    }),
                    ..Default::default()
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }
    fn display_name(&self) -> &str {
        "Web Search"
    }
    fn description(&self) -> &str {
        "Search the web for information using DuckDuckGo."
    }
    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query" },
                "num_results": { "type": "integer", "description": "Number of results (default: 5)", "minimum": 1, "maximum": 20 }
            },
            "required": ["query"]
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let query = params
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .to_string();
        let num_results = params
            .get("num_results")
            .and_then(|n| n.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.default_results);

        Ok(Box::new(WebSearchInvocation {
            query,
            num_results,
            tool: WebSearchTool {
                default_results: self.default_results,
            },
        }))
    }
}
