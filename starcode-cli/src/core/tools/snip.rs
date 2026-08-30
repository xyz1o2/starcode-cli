use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

// ── Snippet Data Model ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snippet {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub language: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnipStore {
    pub snippets: Vec<Snippet>,
}

fn snips_file_path(project_root: &std::path::Path) -> PathBuf {
    project_root.join(".star").join("snips.json")
}

async fn load_store(project_root: &std::path::Path) -> Result<SnipStore, String> {
    let path = snips_file_path(project_root);
    if !path.exists() {
        return Ok(SnipStore::default());
    }
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(SnipStore::default());
    }
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
}

async fn save_store(project_root: &std::path::Path, store: &SnipStore) -> Result<(), String> {
    let path = snips_file_path(project_root);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
        }
    }
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize snip store: {}", e))?;
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

// ── SnipTool ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SnipTool {
    config: Arc<crate::core::config::Config>,
}

impl SnipTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SnipParams {
    pub action: String, // "save" | "list" | "get" | "delete"
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

pub struct SnipInvocation {
    config: Arc<crate::core::config::Config>,
    params: SnipParams,
}

impl ToolInvocation for SnipInvocation {
    fn get_description(&self) -> String {
        match self.params.action.as_str() {
            "save" => format!(
                "Save snippet '{}'",
                self.params.name.as_deref().unwrap_or("unnamed")
            ),
            "get" => format!(
                "Get snippet '{}'",
                self.params.name.as_deref().unwrap_or("?")
            ),
            "list" => "List snippets".to_string(),
            "delete" => format!(
                "Delete snippet '{}'",
                self.params.name.as_deref().unwrap_or("?")
            ),
            _ => format!("Snip action: {}", self.params.action),
        }
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
        let config = self.config.clone();
        let params = self.params.clone();
        Box::pin(async move {
            let root = config.project_root();
            match params.action.as_str() {
                "save" => {
                    let name = match &params.name {
                        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
                        _ => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "invalid_params".to_string(),
                                    message: "name is required for save action".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };
                    let content = params.content.clone().unwrap_or_default();
                    let language = params.language.clone();

                    let mut store = match load_store(&root).await {
                        Ok(s) => s,
                        Err(e) => {
                            return Ok(ToolResult {
                                error: Some(ToolError {
                                    error_type: "snip_error".to_string(),
                                    message: e,
                                }),
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                data: None,
                            })
                        }
                    };

                    let now = Utc::now().timestamp();
                    if let Some(existing) = store.snippets.iter_mut().find(|s| s.name == name) {
                        existing.content = content.clone();
                        existing.language = language.clone();
                        existing.updated_at = now;
                    } else {
                        store.snippets.push(Snippet {
                            name: name.clone(),
                            content,
                            language,
                            created_at: now,
                            updated_at: now,
                        });
                    }

                    if let Err(e) = save_store(&root, &store).await {
                        return Ok(ToolResult {
                            error: Some(ToolError {
                                error_type: "snip_error".to_string(),
                                message: e,
                            }),
                            ..Default::default()
                        });
                    }

                    Ok(ToolResult {
                        llm_content: Some(format!("Snippet '{}' saved.", name)),
                        return_display: Some(format!("Snippet '{}' saved", name)),
                        output: format!("Snippet '{}' saved.", name),
                        error: None,
                        data: Some(serde_json::json!({"status": "saved", "name": name})),
                    })
                }
                "get" => {
                    let name = match &params.name {
                        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
                        _ => {
                            return Ok(ToolResult {
                                error: Some(ToolError {
                                    error_type: "invalid_params".to_string(),
                                    message: "name is required for get action".to_string(),
                                }),
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                data: None,
                            })
                        }
                    };
                    let store = match load_store(&root).await {
                        Ok(s) => s,
                        Err(e) => {
                            return Ok(ToolResult {
                                error: Some(ToolError {
                                    error_type: "snip_error".to_string(),
                                    message: e,
                                }),
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                data: None,
                            })
                        }
                    };
                    match store.snippets.iter().find(|s| s.name == name) {
                        Some(snippet) => {
                            let lang = snippet
                                .language
                                .as_deref()
                                .unwrap_or("text");
                            let display = format!("```{}\n{}\n```", lang, snippet.content);
                            Ok(ToolResult {
                                llm_content: Some(display.clone()),
                                return_display: Some(format!("Snippet '{}'", name)),
                                output: display,
                                error: None,
                                data: Some(serde_json::to_value(snippet).unwrap_or_default()),
                            })
                        }
                        None => Ok(ToolResult {
                            error: Some(ToolError {
                                error_type: "not_found".to_string(),
                                message: format!("Snippet '{}' not found", name),
                            }),
                            ..Default::default()
                        }),
                    }
                }
                "list" => {
                    let store = match load_store(&root).await {
                        Ok(s) => s,
                        Err(e) => {
                            return Ok(ToolResult {
                                error: Some(ToolError {
                                    error_type: "snip_error".to_string(),
                                    message: e,
                                }),
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                data: None,
                            })
                        }
                    };
                    if store.snippets.is_empty() {
                        Ok(ToolResult {
                            llm_content: Some("(no snippets)".to_string()),
                            return_display: Some("No snippets".to_string()),
                            output: "(no snippets)".to_string(),
                            error: None,
                            data: Some(serde_json::json!([])),
                        })
                    } else {
                        let text = store
                            .snippets
                            .iter()
                            .map(|s| {
                                let lang = s.language.as_deref().unwrap_or("text");
                                format!(
                                    "- **{}** ({}, {}B, updated {})",
                                    s.name,
                                    lang,
                                    s.content.len(),
                                    s.updated_at
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        Ok(ToolResult {
                            llm_content: Some(text.clone()),
                            return_display: Some(format!("{} snippets", store.snippets.len())),
                            output: text,
                            error: None,
                            data: Some(
                                serde_json::to_value(&store.snippets).unwrap_or_default(),
                            ),
                        })
                    }
                }
                "delete" => {
                    let name = match &params.name {
                        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
                        _ => {
                            return Ok(ToolResult {
                                error: Some(ToolError {
                                    error_type: "invalid_params".to_string(),
                                    message: "name is required for delete action".to_string(),
                                }),
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                data: None,
                            })
                        }
                    };
                    let mut store = match load_store(&root).await {
                        Ok(s) => s,
                        Err(e) => {
                            return Ok(ToolResult {
                                error: Some(ToolError {
                                    error_type: "snip_error".to_string(),
                                    message: e,
                                }),
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                data: None,
                            })
                        }
                    };
                    let before = store.snippets.len();
                    store.snippets.retain(|s| s.name != name);
                    if store.snippets.len() == before {
                        return Ok(ToolResult {
                            error: Some(ToolError {
                                error_type: "not_found".to_string(),
                                message: format!("Snippet '{}' not found", name),
                            }),
                            ..Default::default()
                        });
                    }
                    if let Err(e) = save_store(&root, &store).await {
                        return Ok(ToolResult {
                            error: Some(ToolError {
                                error_type: "snip_error".to_string(),
                                message: e,
                            }),
                            ..Default::default()
                        });
                    }
                    Ok(ToolResult {
                        llm_content: Some(format!("Snippet '{}' deleted.", name)),
                        return_display: Some(format!("Snippet '{}' deleted", name)),
                        output: format!("Snippet '{}' deleted.", name),
                        error: None,
                        data: Some(serde_json::json!({"status": "deleted", "name": name})),
                    })
                }
                _ => Ok(ToolResult {
                    error: Some(ToolError {
                        error_type: "invalid_action".to_string(),
                        message: format!(
                            "Unknown action '{}'. Valid actions: save, get, list, delete",
                            params.action
                        ),
                    }),
                    ..Default::default()
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for SnipTool {
    fn name(&self) -> &str {
        "snip"
    }

    fn display_name(&self) -> &str {
        "Snip"
    }

    fn description(&self) -> &str {
        "管理代码片段：保存、获取、列表、删除。(Manage code snippets: save, get, list, delete. Snippets are stored in .star/snips.json.)"
    }

    fn kind(&self) -> Kind {
        Kind::Edit
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "get", "list", "delete"],
                    "description": "操作类型: save(保存), get(获取), list(列表), delete(删除)"
                },
                "name": {
                    "type": "string",
                    "description": "片段名称 (Snippet name)"
                },
                "content": {
                    "type": "string",
                    "description": "片段内容，save 时必需 (Snippet content, required for save)"
                },
                "language": {
                    "type": "string",
                    "description": "编程语言 (Programming language hint, e.g. rust, python, javascript)"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SnipParams = serde_json::from_value(params)?;
        Ok(Box::new(SnipInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}
