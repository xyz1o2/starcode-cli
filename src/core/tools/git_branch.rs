use crate::core::tools::git_utils::run_git;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct GitBranchTool {
    config: Arc<crate::core::config::Config>,
}

impl GitBranchTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct GitBranchParams {
    pub action: String,
    pub name: Option<String>,
    pub target: Option<String>,
}

pub struct GitBranchInvocation {
    config: Arc<crate::core::config::Config>,
    params: GitBranchParams,
}

impl ToolInvocation for GitBranchInvocation {
    fn get_description(&self) -> String {
        format!(
            "Git branch {}: {}",
            self.params.action,
            self.params.name.as_deref().unwrap_or("N/A")
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
                "list" => {
                    let output = match run_git(root, &["branch", "--all"]).await {
                        Ok(o) => o,
                        Err(e) => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "git_error".to_string(),
                                    message: format!("Failed to list branches: {}", e),
                                }),
                                data: None,
                            });
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some("Listing git branches".to_string()),
                        return_display: Some("List git branches".to_string()),
                        output,
                        error: None,
                        data: None,
                    })
                }
                "create" => {
                    let name = match params.name {
                        Some(n) => n,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "missing_parameter".to_string(),
                                    message: "Missing branch name".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };
                    let output = match run_git(root, &["checkout", "-b", &name]).await {
                        Ok(o) => o,
                        Err(e) => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "git_error".to_string(),
                                    message: format!("Failed to create branch: {}", e),
                                }),
                                data: None,
                            });
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some(format!("Created branch: {}", name)),
                        return_display: Some(format!("Create branch: {}", name)),
                        output,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "create",
                            "branch": name
                        })),
                    })
                }
                "switch" => {
                    let name = match params.name {
                        Some(n) => n,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "missing_parameter".to_string(),
                                    message: "Missing branch name".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };
                    let output = match run_git(root, &["checkout", &name]).await {
                        Ok(o) => o,
                        Err(e) => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "git_error".to_string(),
                                    message: format!("Failed to switch branch: {}", e),
                                }),
                                data: None,
                            });
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some(format!("Switched to branch: {}", name)),
                        return_display: Some(format!("Switch to branch: {}", name)),
                        output,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "switch",
                            "branch": name
                        })),
                    })
                }
                "delete" => {
                    let name = match params.name {
                        Some(n) => n,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "missing_parameter".to_string(),
                                    message: "Missing branch name".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };
                    let output = match run_git(root, &["branch", "-d", &name]).await {
                        Ok(o) => o,
                        Err(e) => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "git_error".to_string(),
                                    message: format!("Failed to delete branch: {}", e),
                                }),
                                data: None,
                            });
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some(format!("Deleted branch: {}", name)),
                        return_display: Some(format!("Delete branch: {}", name)),
                        output,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "delete",
                            "branch": name
                        })),
                    })
                }
                "merge" => {
                    let name = match params.name {
                        Some(n) => n,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "missing_parameter".to_string(),
                                    message: "Missing branch name".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };
                    let output = match run_git(root, &["merge", &name]).await {
                        Ok(o) => o,
                        Err(e) => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "git_error".to_string(),
                                    message: format!("Failed to merge branch: {}", e),
                                }),
                                data: None,
                            });
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some(format!("Merged branch: {}", name)),
                        return_display: Some(format!("Merge branch: {}", name)),
                        output,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "merge",
                            "branch": name
                        })),
                    })
                }
                "rebase" => {
                    let name = match params.name {
                        Some(n) => n,
                        None => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "missing_parameter".to_string(),
                                    message: "Missing branch name".to_string(),
                                }),
                                data: None,
                            });
                        }
                    };
                    let output = match run_git(root, &["rebase", &name]).await {
                        Ok(o) => o,
                        Err(e) => {
                            return Ok(ToolResult {
                                llm_content: None,
                                return_display: None,
                                output: String::new(),
                                error: Some(ToolError {
                                    error_type: "git_error".to_string(),
                                    message: format!("Failed to rebase branch: {}", e),
                                }),
                                data: None,
                            });
                        }
                    };
                    Ok(ToolResult {
                        llm_content: Some(format!("Rebased onto branch: {}", name)),
                        return_display: Some(format!("Rebase onto branch: {}", name)),
                        output,
                        error: None,
                        data: Some(serde_json::json!({
                            "action": "rebase",
                            "branch": name
                        })),
                    })
                }
                _ => Ok(ToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "invalid_action".to_string(),
                        message: format!("Unknown action: {}", params.action),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for GitBranchTool {
    fn name(&self) -> &str {
        "git_branch"
    }

    fn display_name(&self) -> &str {
        "Git Branch"
    }

    fn description(&self) -> &str {
        "管理 Git 分支。支持列出、创建、切换、删除、合并和变基操作。(Manage Git branches. Supports listing, creating, switching, deleting, merging, and rebasing.)"
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "create", "switch", "delete", "merge", "rebase"],
                    "description": "操作类型 (Action type)"
                },
                "name": {
                    "type": "string",
                    "description": "分支名称 (Branch name)"
                },
                "target": {
                    "type": "string",
                    "description": "目标分支 (用于 merge/rebase 操作)"
                }
            },
            "required": ["action"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GitBranchParams = serde_json::from_value(params)?;
        Ok(Box::new(GitBranchInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}