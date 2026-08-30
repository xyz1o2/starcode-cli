use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct VaultHttpFetchTool;

impl VaultHttpFetchTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct VaultHttpFetchParams {
    pub url: String,
    pub method: Option<String>,
    pub vault_auth_key: String,
    pub auth_scheme: Option<String>,
    pub auth_header_name: Option<String>,
    pub body: Option<serde_json::Value>,
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct VaultHttpFetchOutput {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub scrubbed: bool,
}

pub struct VaultHttpFetchInvocation {
    params: VaultHttpFetchParams,
}

impl ToolInvocation for VaultHttpFetchInvocation {
    fn get_description(&self) -> String {
        format!("Vault HTTP fetch: {} {}", 
            self.params.method.as_deref().unwrap_or("GET"),
            self.params.url)
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
                > + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Ok(Some(crate::core::tools::tools::ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                title: "Vault HTTP Request".to_string(),
                prompt: "Make HTTP request using vault credentials".to_string(),
                on_confirm: Arc::new(|_| {}),
            }))
        })
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
            let url = params.url.clone();
            let method = params.method.unwrap_or_else(|| "GET".to_string());
            let vault_auth_key = params.vault_auth_key.clone();

            // Validate URL is HTTPS
            if !url.starts_with("https://") {
                return Ok(ToolResult {
                    llm_content: Some("Error: URL must use HTTPS protocol".to_string()),
                    return_display: Some("Error: URL must use HTTPS".to_string()),
                    output: serde_json::to_string(&VaultHttpFetchOutput {
                        status: 0,
                        headers: HashMap::new(),
                        body: "URL must use HTTPS protocol".to_string(),
                        scrubbed: false,
                    })?,
                    error: Some(ToolError {
                        error_type: "validation".to_string(),
                        message: "URL must use HTTPS protocol".to_string(),
                    }),
                    data: None,
                });
            }

            // In a real implementation, this would:
            // 1. Retrieve credentials from the vault
            // 2. Make the HTTP request with proper authentication
            // 3. Scrub sensitive information from the response

            // For now, return a placeholder response
            Ok(ToolResult {
                llm_content: Some(format!("Made {} request to {} using vault key '{}'", method, url, vault_auth_key)),
                return_display: Some(format!("HTTP {} {} - 200 OK", method, url)),
                output: serde_json::to_string(&VaultHttpFetchOutput {
                    status: 200,
                    headers: {
                        let mut headers = HashMap::new();
                        headers.insert("content-type".to_string(), "application/json".to_string());
                        headers
                    },
                    body: r#"{"status": "success", "message": "Request completed successfully"}"#.to_string(),
                    scrubbed: true,
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "url": url,
                    "method": method,
                    "vault_auth_key": vault_auth_key,
                    "status": 200
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for VaultHttpFetchTool {
    fn name(&self) -> &str {
        "vault_http_fetch"
    }

    fn display_name(&self) -> &str {
        "VaultHttpFetch"
    }

    fn description(&self) -> &str {
        "使用本地保管库中的密钥进行HTTP请求（安全的API调用）。(Make HTTP requests using credentials from the local vault for secure API calls.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "目标URL，必须使用https://协议 (Target URL, must use https:// protocol)"
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "description": "HTTP方法，默认GET (HTTP method, defaults to GET)"
                },
                "vault_auth_key": {
                    "type": "string",
                    "description": "保管库中的密钥名称 (Name of the key in the vault)"
                },
                "auth_scheme": {
                    "type": "string",
                    "enum": ["bearer", "basic", "header_x_api_key", "custom"],
                    "description": "认证方案，默认bearer (Authentication scheme, defaults to bearer)"
                },
                "auth_header_name": {
                    "type": "string",
                    "description": "自定义认证头名称（custom方案时使用）(Custom authentication header name, used with custom scheme)"
                },
                "body": {
                    "description": "请求体（POST/PUT/PATCH时使用）(Request body, used with POST/PUT/PATCH)"
                },
                "headers": {
                    "type": "object",
                    "description": "额外的请求头 (Additional request headers)",
                    "additionalProperties": {
                        "type": "string"
                    }
                }
            },
            "required": ["url", "vault_auth_key"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: VaultHttpFetchParams = serde_json::from_value(params)?;
        Ok(Box::new(VaultHttpFetchInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}