use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use html2text;
use readability_rust::Readability;
use reqwest;
use serde::Deserialize;

#[derive(Clone)]
pub struct WebFetchTool;

impl WebFetchTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
pub struct WebFetchParams {
    pub url: String,
}

pub struct WebFetchInvocation {
    params: WebFetchParams,
}

impl ToolInvocation for WebFetchInvocation {
    fn get_description(&self) -> String {
        format!("Fetch content from {}", self.params.url)
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
        let url = self.params.url.clone();

        Box::pin(async move {
            // 离线模式：WebFetch 直接拒绝（对标 Claude Code /network）
            if crate::core::offline::is_offline() {
                crate::utils::logging::append_debug_log_line(
                    "[web_fetch] Refusing fetch (offline mode)",
                );
                return Ok(ToolResult {
                    llm_content: Some(
                        "Web fetch is unavailable because offline mode is ON. \
                         Turn it off with /network off before fetching URLs."
                            .to_string(),
                    ),
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "offline".to_string(),
                        message: "Web fetch refused (offline mode)".to_string(),
                    }),
                    data: None,
                });
            }

            // 1. Fetch HTML
            let client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
                .timeout(std::time::Duration::from_secs(30))
                .build()?;

            let response = client.get(&url).send().await?;

            if !response.status().is_success() {
                return Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "http_error".to_string(),
                        message: format!("HTTP error: {}", response.status()),
                    }),
                    data: None,
                });
            }

            let html_content = response.text().await?;

            // 2. Extract Content using Readability
            // readability-rust uses synchronous parsing, so we might block the async thread briefly.
            let parser_result = Readability::new(&html_content, None);

            let article = match parser_result {
                Ok(mut parser) => parser.parse(),
                Err(_) => None, // Failed to init parser
            };

            if let Some(article) = article {
                // 3. Convert Extracted HTML to Markdown
                let title = article.title.unwrap_or_default();
                let byline = article.byline.unwrap_or_default();
                let excerpt = article.excerpt.unwrap_or_default();
                let content_html = article.content.unwrap_or_default();

                let markdown_body = html2text::from_read(content_html.as_bytes(), 80)
                    .unwrap_or_else(|_| content_html.clone());

                let final_output = format!(
                    "# {}\n\n**Author:** {}\n**Excerpt:** {}\n\n---\n\n{}",
                    title, byline, excerpt, markdown_body
                );

                Ok(ToolResult {
                    llm_content: Some(final_output.clone()),
                    return_display: Some(final_output.clone()),
                    output: final_output,
                    error: None,
                    data: None,
                })
            } else {
                // Fallback: If readability fails, just return raw text via html2text on the full body
                let text = html2text::from_read(html_content.as_bytes(), 80)
                    .unwrap_or_else(|e| format!("HTML parsing failed: {}", e));
                Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: format!(
                        "Readability failed to extract article. Returning raw text fallback:\n\n{}",
                        text
                    ),
                    error: None,
                    data: None,
                })
            }
        })
    }
}

impl BaseDeclarativeTool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn display_name(&self) -> &str {
        "Web Fetch"
    }

    fn description(&self) -> &str {
        "Fetch and extract text content from a URL. Uses readability mode to extract main article content."
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch."
                }
            },
            "required": ["url"]
        })
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WebFetchParams = serde_json::from_value(params)?;
        Ok(Box::new(WebFetchInvocation { params }))
    }
}
