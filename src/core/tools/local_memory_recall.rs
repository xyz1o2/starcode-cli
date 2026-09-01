use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct LocalMemoryRecallTool;

impl LocalMemoryRecallTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct LocalMemoryRecallParams {
    pub action: String,
    pub store: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct LocalMemoryRecallOutput {
    pub entries: Vec<MemoryEntry>,
    pub stores: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub store: String,
}

pub struct LocalMemoryRecallInvocation {
    params: LocalMemoryRecallParams,
}

impl ToolInvocation for LocalMemoryRecallInvocation {
    fn get_description(&self) -> String {
        format!("Local memory recall: {}", self.params.action)
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

            match action.as_str() {
                "get" => {
                    let key = params.key.ok_or("key is required for get action")?;
                    let store = params.store.unwrap_or_else(|| "default".to_string());

                    // In a real implementation, this would read from the memory store
                    Ok(ToolResult {
                        llm_content: Some(format!(
                            "Retrieved value for key '{}' from store '{}'",
                            key, store
                        )),
                        return_display: Some(format!("Memory retrieved: {}", key)),
                        output: serde_json::to_string(&LocalMemoryRecallOutput {
                            entries: vec![MemoryEntry {
                                key: key.clone(),
                                value: "example_value".to_string(),
                                store: store.clone(),
                            }],
                            stores: None,
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "get",
                            "key": key,
                            "store": store
                        })),
                    })
                }
                "list" => {
                    let store = params.store.unwrap_or_else(|| "default".to_string());

                    // In a real implementation, this would list all entries in the store
                    Ok(ToolResult {
                        llm_content: Some(format!("Listed entries in store '{}'", store)),
                        return_display: Some(format!("Memory entries listed")),
                        output: serde_json::to_string(&LocalMemoryRecallOutput {
                            entries: vec![
                                MemoryEntry {
                                    key: "example_key_1".to_string(),
                                    value: "example_value_1".to_string(),
                                    store: store.clone(),
                                },
                                MemoryEntry {
                                    key: "example_key_2".to_string(),
                                    value: "example_value_2".to_string(),
                                    store: store.clone(),
                                },
                            ],
                            stores: None,
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "list",
                            "store": store
                        })),
                    })
                }
                "list_stores" => {
                    // In a real implementation, this would list all available stores
                    Ok(ToolResult {
                        llm_content: Some("Listed all memory stores".to_string()),
                        return_display: Some("Memory stores listed".to_string()),
                        output: serde_json::to_string(&LocalMemoryRecallOutput {
                            entries: vec![],
                            stores: Some(vec![
                                "default".to_string(),
                                "session".to_string(),
                                "project".to_string(),
                            ]),
                        })?,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "list_stores"
                        })),
                    })
                }
                _ => Ok(ToolResult {
                    llm_content: Some(format!("Unknown action: {}", action)),
                    return_display: Some(format!("Unknown action: {}", action)),
                    output: serde_json::to_string(&LocalMemoryRecallOutput {
                        entries: vec![],
                        stores: None,
                    })?,
                    error: Some(ToolError {
                        error_type: "validation".to_string(),
                        message: format!(
                            "Unknown action: {}. Use 'get', 'list', or 'list_stores'",
                            action
                        ),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for LocalMemoryRecallTool {
    fn name(&self) -> &str {
        "local_memory_recall"
    }

    fn display_name(&self) -> &str {
        "LocalMemoryRecall"
    }

    fn description(&self) -> &str {
        "从本地会话记忆存储中检索和列出记忆条目。(Retrieve and list memory entries from local session memory storage.)"
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
                    "enum": ["get", "list", "list_stores"],
                    "description": "要执行的操作：'get' 获取单个条目，'list' 列出存储中的所有条目，'list_stores' 列出所有存储"
                },
                "store": {
                    "type": "string",
                    "description": "存储名称，默认为 'default' (Store name, defaults to 'default')"
                },
                "key": {
                    "type": "string",
                    "description": "要获取的条目的键名（get操作必填）(Key of the entry to retrieve, required for get action)"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: LocalMemoryRecallParams = serde_json::from_value(params)?;
        Ok(Box::new(LocalMemoryRecallInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
