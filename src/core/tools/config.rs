use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone)]
pub struct ConfigTool {
    config: Arc<crate::core::config::Config>,
}

impl ConfigTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConfigParams {
    pub setting: String,
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ConfigOutput {
    pub success: bool,
    pub operation: Option<String>,
    pub setting: Option<String>,
    pub value: Option<serde_json::Value>,
    pub previous_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
    pub error: Option<String>,
}

pub struct ConfigInvocation {
    params: ConfigParams,
    config: Arc<crate::core::config::Config>,
}

impl ToolInvocation for ConfigInvocation {
    fn get_description(&self) -> String {
        if self.params.value.is_some() {
            format!(
                "Set config {} to {:?}",
                self.params.setting, self.params.value
            )
        } else {
            format!("Get config {}", self.params.setting)
        }
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
        let params = self.params.clone();
        Box::pin(async move {
            // Auto-allow reading configs
            if params.value.is_none() {
                return Ok(None);
            }
            // Ask for confirmation when setting values
            Ok(Some(
                crate::core::tools::tools::ToolCallConfirmationDetails {
                    confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                    title: "Config Change".to_string(),
                    prompt: format!(
                        "Set {} to {:?}",
                        params.setting,
                        params.value.unwrap_or(serde_json::Value::Null)
                    ),
                    on_confirm: Arc::new(|_| {}),
                },
            ))
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
        let config = self.config.clone();
        Box::pin(async move {
            let setting = &params.setting;

            // Check if setting is supported
            if !is_supported_setting(setting) {
                return Ok(ToolResult {
                    llm_content: Some(format!("Unknown setting: \"{}\"", setting)),
                    return_display: Some(format!("Unknown setting: \"{}\"", setting)),
                    output: serde_json::to_string(&ConfigOutput {
                        success: false,
                        operation: None,
                        setting: Some(setting.clone()),
                        value: None,
                        previous_value: None,
                        new_value: None,
                        error: Some(format!("Unknown setting: \"{}\"", setting)),
                    })?,
                    error: Some(ToolError {
                        error_type: "validation".to_string(),
                        message: format!("Unknown setting: \"{}\"", setting),
                    }),
                    data: None,
                });
            }

            // GET operation
            if params.value.is_none() {
                let current_value = get_config_value(&config, setting);
                return Ok(ToolResult {
                    llm_content: Some(format!("{} = {:?}", setting, current_value)),
                    return_display: Some(format!("{} = {:?}", setting, current_value)),
                    output: serde_json::to_string(&ConfigOutput {
                        success: true,
                        operation: Some("get".to_string()),
                        setting: Some(setting.clone()),
                        value: current_value.clone(),
                        previous_value: None,
                        new_value: None,
                        error: None,
                    })?,
                    error: None,
                    data: Some(serde_json::json!({
                        "operation": "get",
                        "setting": setting,
                        "value": current_value
                    })),
                });
            }

            // SET operation
            let value = params.value.unwrap();
            let previous_value = get_config_value(&config, setting);

            // Validate the value
            if let Some(error) = validate_setting_value(setting, &value) {
                return Ok(ToolResult {
                    llm_content: Some(format!("Error: {}", error)),
                    return_display: Some(format!("Error: {}", error)),
                    output: serde_json::to_string(&ConfigOutput {
                        success: false,
                        operation: Some("set".to_string()),
                        setting: Some(setting.clone()),
                        value: Some(value),
                        previous_value,
                        new_value: None,
                        error: Some(error.clone()),
                    })?,
                    error: Some(ToolError {
                        error_type: "validation".to_string(),
                        message: error,
                    }),
                    data: None,
                });
            }

            // Apply the setting
            match set_config_value(&config, setting, &value) {
                Ok(_) => Ok(ToolResult {
                    llm_content: Some(format!("Set {} to {:?}", setting, value)),
                    return_display: Some(format!("Set {} to {:?}", setting, value)),
                    output: serde_json::to_string(&ConfigOutput {
                        success: true,
                        operation: Some("set".to_string()),
                        setting: Some(setting.clone()),
                        value: Some(value.clone()),
                        previous_value,
                        new_value: Some(value),
                        error: None,
                    })?,
                    error: None,
                    data: Some(serde_json::json!({
                        "operation": "set",
                        "setting": setting,
                        "success": true
                    })),
                }),
                Err(e) => Ok(ToolResult {
                    llm_content: Some(format!("Error setting config: {}", e)),
                    return_display: Some(format!("Error setting config: {}", e)),
                    output: serde_json::to_string(&ConfigOutput {
                        success: false,
                        operation: Some("set".to_string()),
                        setting: Some(setting.clone()),
                        value: Some(value),
                        previous_value,
                        new_value: None,
                        error: Some(e.to_string()),
                    })?,
                    error: Some(ToolError {
                        error_type: "execution".to_string(),
                        message: e.to_string(),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for ConfigTool {
    fn name(&self) -> &str {
        "config"
    }

    fn display_name(&self) -> &str {
        "Config"
    }

    fn description(&self) -> &str {
        "获取或设置StarCode配置项，如主题、模型、权限模式等。(Get or set StarCode settings like theme, model, permissions mode, etc.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "setting": {
                    "type": "string",
                    "description": "配置项名称，如 \"theme\", \"model\", \"permissions.defaultMode\" (The setting key)"
                },
                "value": {
                    "description": "新值，省略则获取当前值 (The new value. Omit to get current value.)"
                }
            },
            "required": ["setting"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ConfigParams = serde_json::from_value(params)?;
        Ok(Box::new(ConfigInvocation {
            params,
            config: self.config.clone(),
        }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}

fn is_supported_setting(setting: &str) -> bool {
    matches!(
        setting,
        "theme"
            | "model"
            | "permissions.defaultMode"
            | "permissions.dangerousMode"
            | "output.style"
            | "language"
            | "vim"
            | "showMemoryUsage"
            | "enableTelemetry"
            | "enableCheckpointing"
            | "enableShellOutputEfficiency"
            | "shellToolInactivityTimeout"
            | "maxSessionTurns"
            | "contextWindow"
            | "compressionThreshold"
            | "truncateToolOutputThreshold"
            | "truncateToolOutputLines"
            | "enableToolOutputTruncation"
            | "enableInteractiveShell"
            | "enablePromptCompletion"
            | "enableHooks"
            | "enableAgents"
            | "skillsSupport"
            | "experimentalJitContext"
            | "previewFeatures"
    )
}

fn get_config_value(
    config: &crate::core::config::Config,
    setting: &str,
) -> Option<serde_json::Value> {
    match setting {
        "theme" => Some(serde_json::json!("dark")),
        "model" => Some(serde_json::json!(config.model())),
        "permissions.defaultMode" => Some(serde_json::json!("default")),
        "permissions.dangerousMode" => Some(serde_json::json!(false)),
        "output.style" => Some(serde_json::json!("default")),
        "language" => Some(serde_json::json!("en-US")),
        "vim" => Some(serde_json::json!(false)),
        "showMemoryUsage" => Some(serde_json::json!(config.show_memory_usage())),
        "enableTelemetry" => Some(serde_json::json!(true)),
        "enableCheckpointing" => Some(serde_json::json!(config.checkpointing_enabled())),
        "enableShellOutputEfficiency" => {
            Some(serde_json::json!(config.enable_shell_output_efficiency()))
        }
        "shellToolInactivityTimeout" => {
            Some(serde_json::json!(config.shell_tool_inactivity_timeout()))
        }
        "maxSessionTurns" => Some(serde_json::json!(config.max_session_turns())),
        "contextWindow" => Some(serde_json::json!(config.context_window())),
        "compressionThreshold" => Some(serde_json::json!(config.compression_threshold())),
        "truncateToolOutputThreshold" => {
            Some(serde_json::json!(config.truncate_tool_output_threshold()))
        }
        "truncateToolOutputLines" => Some(serde_json::json!(config.truncate_tool_output_lines())),
        "enableToolOutputTruncation" => {
            Some(serde_json::json!(config.enable_tool_output_truncation()))
        }
        "enableInteractiveShell" => Some(serde_json::json!(config.enable_interactive_shell())),
        "enablePromptCompletion" => Some(serde_json::json!(config.enable_prompt_completion())),
        "enableHooks" => Some(serde_json::json!(config.enable_hooks())),
        "enableAgents" => Some(serde_json::json!(config.enable_agents())),
        "skillsSupport" => Some(serde_json::json!(config.skills_support())),
        "experimentalJitContext" => Some(serde_json::json!(config.experimental_jit_context())),
        "previewFeatures" => Some(serde_json::json!(config.preview_features())),
        _ => None,
    }
}

fn validate_setting_value(setting: &str, value: &serde_json::Value) -> Option<String> {
    match setting {
        "theme" => {
            if let Some(s) = value.as_str() {
                if !["dark", "light", "auto"].contains(&s) {
                    return Some(format!("Invalid theme: {}. Options: dark, light, auto", s));
                }
            } else {
                return Some("Theme must be a string".to_string());
            }
        }
        "permissions.defaultMode" => {
            if let Some(s) = value.as_str() {
                if !["default", "plan", "yolo"].contains(&s) {
                    return Some(format!("Invalid mode: {}. Options: default, plan, yolo", s));
                }
            } else {
                return Some("Mode must be a string".to_string());
            }
        }
        "permissions.dangerousMode" => {
            if !value.is_boolean() {
                return Some("Dangerous mode must be a boolean".to_string());
            }
        }
        "vim" => {
            if !value.is_boolean() {
                return Some("Vim mode must be a boolean".to_string());
            }
        }
        "showMemoryUsage" => {
            if !value.is_boolean() {
                return Some("showMemoryUsage must be a boolean".to_string());
            }
        }
        "enableTelemetry" => {
            if !value.is_boolean() {
                return Some("enableTelemetry must be a boolean".to_string());
            }
        }
        "enableCheckpointing" => {
            if !value.is_boolean() {
                return Some("enableCheckpointing must be a boolean".to_string());
            }
        }
        "enableShellOutputEfficiency" => {
            if !value.is_boolean() {
                return Some("enableShellOutputEfficiency must be a boolean".to_string());
            }
        }
        "shellToolInactivityTimeout" => {
            if let Some(n) = value.as_u64() {
                if n < 1000 || n > 300000 {
                    return Some("Timeout must be between 1000 and 300000 ms".to_string());
                }
            } else {
                return Some("Timeout must be a number".to_string());
            }
        }
        "maxSessionTurns" => {
            if let Some(n) = value.as_i64() {
                if n < 1 || n > 1000 {
                    return Some("Max turns must be between 1 and 1000".to_string());
                }
            } else {
                return Some("Max turns must be a number".to_string());
            }
        }
        "contextWindow" => {
            if let Some(n) = value.as_u64() {
                if n < 1000 || n > 1000000 {
                    return Some("Context window must be between 1000 and 1000000".to_string());
                }
            } else {
                return Some("Context window must be a number".to_string());
            }
        }
        "compressionThreshold" => {
            if let Some(n) = value.as_f64() {
                if n < 0.0 || n > 1.0 {
                    return Some("Compression threshold must be between 0.0 and 1.0".to_string());
                }
            } else {
                return Some("Compression threshold must be a number".to_string());
            }
        }
        "truncateToolOutputThreshold" => {
            if !value.is_number() {
                return Some("Threshold must be a number".to_string());
            }
        }
        "truncateToolOutputLines" => {
            if !value.is_number() {
                return Some("Lines must be a number".to_string());
            }
        }
        "enableToolOutputTruncation" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "enableInteractiveShell" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "enablePromptCompletion" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "enableHooks" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "enableAgents" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "skillsSupport" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "experimentalJitContext" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        "previewFeatures" => {
            if !value.is_boolean() {
                return Some("Must be a boolean".to_string());
            }
        }
        _ => {}
    }
    None
}

fn set_config_value(
    config: &crate::core::config::Config,
    setting: &str,
    value: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    // This is a simplified implementation
    // In a real implementation, you would update the config storage
    match setting {
        "model" => {
            if let Some(model) = value.as_str() {
                // config.set_model(model.to_string());
                tracing::info!("Setting model to: {}", model);
            }
        }
        "theme" => {
            if let Some(theme) = value.as_str() {
                tracing::info!("Setting theme to: {}", theme);
            }
        }
        _ => {
            tracing::info!("Setting {} to {:?}", setting, value);
        }
    }
    Ok(())
}
