use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebBrowserToolParams {
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub value: Option<String>,
    #[serde(default = "default_format")]
    pub format: String,
}

fn default_format() -> String {
    "text".to_string()
}

#[derive(Clone)]
pub struct WebBrowserTool {
    client: Option<reqwest::Client>,
}

impl WebBrowserTool {
    pub fn new() -> Self {
        Self {
            client: Some(reqwest::Client::new()),
        }
    }
}

pub struct WebBrowserToolInvocation {
    params: WebBrowserToolParams,
    client: Option<reqwest::Client>,
}

impl WebBrowserToolInvocation {
    pub fn new(params: WebBrowserToolParams, client: Option<reqwest::Client>) -> Self {
        Self { params, client }
    }
}

impl BaseDeclarativeTool for WebBrowserTool {
    fn name(&self) -> &str {
        "web_browser"
    }

    fn display_name(&self) -> &str {
        "Web Browser"
    }

    fn description(&self) -> &str {
        "Interact with web pages - navigate, click, fill forms, extract content. Supports headless browsing for web scraping and automation."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "click", "fill", "extract", "screenshot"],
                    "description": "Action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for navigate action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for element to interact with"
                },
                "value": {
                    "type": "string",
                    "description": "Value to fill in form field (for fill action)"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "html", "markdown"],
                    "description": "Output format for extract action",
                    "default": "text"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: WebBrowserToolParams = serde_json::from_value(params)?;
        Ok(Box::new(WebBrowserToolInvocation::new(
            params,
            self.client.clone(),
        )))
    }
}

impl ToolInvocation for WebBrowserToolInvocation {
    fn get_description(&self) -> String {
        format!(
            "Web Browser: {} {}",
            self.params.action,
            self.params.url.as_deref().unwrap_or("")
        )
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
        let action = self.params.action.clone();
        let url = self.params.url.clone();

        Box::pin(async move {
            if action == "navigate" {
                if let Some(url) = &url {
                    if url.starts_with("file://")
                        || url.contains("localhost")
                        || url.contains("127.0.0.1")
                    {
                        return Ok(Some(
                            crate::core::tools::tools::ToolCallConfirmationDetails {
                                confirmation_type:
                                    crate::core::tools::tools::ConfirmationType::Warning,
                                title: "Local Navigation".to_string(),
                                prompt: format!("Navigating to local URL: {}. Proceed?", url),
                                on_confirm: std::sync::Arc::new(|_| {}),
                            },
                        ));
                    }
                }
            }

            Ok(None)
        })
    }

    fn execute(
        &self,
        signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        let client = self.client.clone();
        let signal = signal.cloned();

        Box::pin(async move {
            if let Some(signal) = &signal {
                if signal.is_cancelled() {
                    return Ok(ToolResult {
                        llm_content: Some("Operation was cancelled by user.".to_string()),
                        return_display: Some("Operation cancelled.".to_string()),
                        output: "Operation cancelled by user.".to_string(),
                        error: None,
                        data: None,
                    });
                }
            }

            match params.action.as_str() {
                "navigate" => {
                    let url = params
                        .url
                        .as_ref()
                        .ok_or("Missing required parameter: url for navigate")?;

                    let client = client.ok_or("HTTP client not available")?;

                    let response = client
                        .get(url)
                        .send()
                        .await
                        .map_err(|e| format!("Failed to navigate: {}", e))?;

                    let status = response.status();
                    let headers = response.headers().clone();
                    let body = response
                        .text()
                        .await
                        .map_err(|e| format!("Failed to read response: {}", e))?;

                    let llm_content = format!(
                        "Navigated to {}\nStatus: {}\nContent-Length: {}\nHeaders: {:?}",
                        url,
                        status,
                        body.len(),
                        headers
                    );

                    Ok(ToolResult {
                        llm_content: Some(llm_content),
                        return_display: Some(format!("Navigated to {} (Status: {})", url, status)),
                        output: body,
                        error: if !status.is_success() {
                            Some(crate::core::tools::tools::ToolError {
                                error_type: "http_error".to_string(),
                                message: format!("HTTP status: {}", status),
                            })
                        } else {
                            None
                        },
                        data: Some(json!({
                            "url": url,
                            "status": status.as_u16(),
                            "headers": headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect::<Vec<_>>()
                        })),
                    })
                }

                "extract" => {
                    let _format = params.format.as_str();
                    let url = params
                        .url
                        .as_ref()
                        .ok_or("Missing required parameter: url for extract")?;

                    let client = client.ok_or("HTTP client not available")?;

                    let response = client
                        .get(url)
                        .send()
                        .await
                        .map_err(|e| format!("Failed to fetch page: {}", e))?;

                    let html = response
                        .text()
                        .await
                        .map_err(|e| format!("Failed to read response: {}", e))?;

                    let text = html2text::from_read(html.as_bytes(), 80)
                        .map_err(|e| format!("Failed to convert HTML to text: {}", e))?;

                    Ok(ToolResult {
                        llm_content: Some(format!("Extracted content from {}", url)),
                        return_display: Some(format!(
                            "Extracted {} characters from {}",
                            text.len(),
                            url
                        )),
                        output: text,
                        error: None,
                        data: None,
                    })
                }

                "screenshot" => {
                    let url = params
                        .url
                        .as_ref()
                        .ok_or("Missing required parameter: url for screenshot")?;

                    Ok(ToolResult {
                        llm_content: Some(format!("Screenshot of {} (placeholder)", url)),
                        return_display: Some(
                            "Screenshot functionality requires headless browser".to_string(),
                        ),
                        output: "Screenshot functionality requires headless browser integration"
                            .to_string(),
                        error: None,
                        data: None,
                    })
                }

                "click" | "fill" => {
                    let selector = params
                        .selector
                        .as_ref()
                        .ok_or("Missing required parameter: selector")?;

                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "{} on {} (placeholder)",
                            params.action, selector
                        )),
                        return_display: Some(
                            "DOM interaction requires headless browser".to_string(),
                        ),
                        output: "DOM interaction requires headless browser integration".to_string(),
                        error: None,
                        data: None,
                    })
                }

                _ => Err(format!("Unknown action: {}", params.action).into()),
            }
        })
    }
}
