use crate::core::tools::constants::GET_DIAGNOSTICS_TOOL_NAME;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::paths::{make_relative, shorten_path};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDiagnosticsParams {
    pub uri: Option<String>,
}

pub struct GetDiagnosticsTool {
    config: Arc<crate::core::config::Config>,
}

impl GetDiagnosticsTool {
    pub fn new(config: Arc<crate::core::config::Config>) -> Self {
        Self { config }
    }
}

impl BaseDeclarativeTool for GetDiagnosticsTool {
    fn name(&self) -> &str {
        GET_DIAGNOSTICS_TOOL_NAME
    }

    fn display_name(&self) -> &str {
        "Get Diagnostics"
    }

    fn description(&self) -> &str {
        "Get language diagnostics (errors, warnings) from the project. Supports Rust (cargo check), Node.js (eslint), and Python (pylint) automatically. Returns a list of diagnostics with file paths, line numbers, and messages."
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "uri": {
                    "type": "string",
                    "description": "Optional file path to filter diagnostics for."
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: GetDiagnosticsParams = serde_json::from_value(params)?;
        Ok(Box::new(GetDiagnosticsInvocation {
            config: self.config.clone(),
            params,
        }))
    }
}

pub struct GetDiagnosticsInvocation {
    config: Arc<crate::core::config::Config>,
    params: GetDiagnosticsParams,
}

// Rust Cargo Types
#[derive(Debug, Serialize, Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<DiagnosticMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiagnosticMessage {
    message: String,
    level: String,
    spans: Vec<DiagnosticSpan>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DiagnosticSpan {
    file_name: String,
    line_start: u64,
    column_start: u64,
    line_end: u64,
    column_end: u64,
    is_primary: bool,
}

// ESLint Types
#[derive(Debug, Serialize, Deserialize)]
struct EslintFile {
    #[serde(rename = "filePath")]
    file_path: String,
    messages: Vec<EslintMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EslintMessage {
    #[serde(rename = "ruleId")]
    rule_id: Option<String>,
    severity: u8, // 1: warning, 2: error
    message: String,
    line: u64,
    column: u64,
}

// Pylint Types
#[derive(Debug, Serialize, Deserialize)]
struct PylintMessage {
    #[serde(rename = "type")]
    msg_type: String,
    path: String,
    line: u64,
    column: u64,
    message: String,
    symbol: String,
}

impl ToolInvocation for GetDiagnosticsInvocation {
    fn get_description(&self) -> String {
        if let Some(uri) = &self.params.uri {
            let relative_path = make_relative(Path::new(uri), &self.config.target_dir());
            format!(
                "Checking diagnostics for {}",
                shorten_path(&relative_path.to_string_lossy(), 80)
            )
        } else {
            "Checking diagnostics for project".to_string()
        }
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        if let Some(uri) = &self.params.uri {
            vec![ToolLocation {
                path: PathBuf::from(uri),
                location_type: crate::core::tools::tools::LocationType::Read,
            }]
        } else {
            vec![]
        }
    }

    fn execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<Arc<dyn Fn(String) + Send + Sync>>,
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
            let result = tokio::task::spawn_blocking(
                move || -> Result<ToolResult, Box<dyn std::error::Error + Send + Sync>> {
                    let root_dir = config.target_dir();
                    let mut all_diagnostics = Vec::new();

                    // 1. Rust Project Detection
                    let cargo_toml = root_dir.join("Cargo.toml");
                    if cargo_toml.exists() {
                        if let Ok(output) = Command::new("cargo")
                            .current_dir(&root_dir)
                            .args(&["check", "--message-format=json"])
                            .output()
                        {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            for line in stdout.lines() {
                                if let Ok(msg) = serde_json::from_str::<CargoMessage>(line) {
                                    if msg.reason == "compiler-message" {
                                        if let Some(dm) = msg.message {
                                            if let Some(ref filter) = params.uri {
                                                let has_file = dm.spans.iter().any(|s| {
                                                    filter.contains(&s.file_name)
                                                        || s.file_name.contains(filter)
                                                });
                                                if !has_file {
                                                    continue;
                                                }
                                            }
                                            let primary_span = dm
                                                .spans
                                                .iter()
                                                .find(|s| s.is_primary)
                                                .or(dm.spans.first());
                                            let location = if let Some(span) = primary_span {
                                                format!(
                                                    "{}:{}:{}",
                                                    span.file_name,
                                                    span.line_start,
                                                    span.column_start
                                                )
                                            } else {
                                                "unknown".to_string()
                                            };
                                            all_diagnostics.push(format!(
                                                "[RUST] [{}] {} - {}",
                                                dm.level.to_uppercase(),
                                                location,
                                                dm.message
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 2. Node.js Project Detection (ESLint)
                    let package_json = root_dir.join("package.json");
                    if package_json.exists() {
                        // Try running npx eslint
                        // Check if node_modules exists to avoid failing if not installed
                        if root_dir.join("node_modules").exists() {
                            let mut cmd = Command::new("npx");
                            cmd.current_dir(&root_dir)
                                .arg("eslint")
                                .arg(".")
                                .arg("--format")
                                .arg("json");

                            // On Windows, npx might be npx.cmd
                            if cfg!(windows) {
                                cmd = Command::new("cmd");
                                cmd.current_dir(&root_dir)
                                    .args(&["/C", "npx", "eslint", ".", "--format", "json"]);
                            }

                            if let Ok(output) = cmd.output() {
                                let stdout = String::from_utf8_lossy(&output.stdout);
                                // ESLint outputs JSON array
                                if let Ok(files) = serde_json::from_str::<Vec<EslintFile>>(&stdout)
                                {
                                    for file in files {
                                        for msg in file.messages {
                                            // Filter logic inside loop to avoid ownership issues if needed.
                                            if let Some(ref filter) = params.uri {
                                                if !file.file_path.contains(filter) {
                                                    continue;
                                                }
                                            }

                                            let rel_path = make_relative(
                                                std::path::Path::new(&file.file_path),
                                                &root_dir,
                                            );
                                            let severity = if msg.severity == 2 {
                                                "ERROR"
                                            } else {
                                                "WARNING"
                                            };
                                            all_diagnostics.push(format!(
                                                "[ESLINT] [{}] {}:{}:{} - {} ({})",
                                                severity,
                                                rel_path.display(),
                                                msg.line,
                                                msg.column,
                                                msg.message,
                                                msg.rule_id.clone().unwrap_or_default()
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 3. Python Project Detection (Pylint)
                    // Detect by presence of requirements.txt or *.py files
                    // For now, simple check for requirements.txt or pyproject.toml
                    let has_python = root_dir.join("requirements.txt").exists()
                        || root_dir.join("pyproject.toml").exists();
                    if has_python {
                        // Try pylint
                        // Assume 'pylint' is in PATH
                        let mut cmd = Command::new("pylint");
                        cmd.current_dir(&root_dir)
                            .arg(".")
                            .arg("--output-format=json")
                            .arg("--recursive=y"); // Pylint 2.13+

                        if let Ok(output) = cmd.output() {
                            let stdout = String::from_utf8_lossy(&output.stdout);
                            if let Ok(msgs) = serde_json::from_str::<Vec<PylintMessage>>(&stdout) {
                                for msg in msgs {
                                    if let Some(ref filter) = params.uri {
                                        if !msg.path.contains(filter) {
                                            continue;
                                        }
                                    }
                                    all_diagnostics.push(format!(
                                        "[PYLINT] [{}] {}:{}:{} - {} ({})",
                                        msg.msg_type.to_uppercase(),
                                        msg.path,
                                        msg.line,
                                        msg.column,
                                        msg.message,
                                        msg.symbol
                                    ));
                                }
                            }
                        }
                    }

                    if all_diagnostics.is_empty() {
                        Ok(ToolResult {
                            output: "No diagnostics found (clean build / no linters detected)."
                                .to_string(),
                            llm_content: None,
                            return_display: None,
                            error: None,
                            data: None,
                        })
                    } else {
                        Ok(ToolResult {
                            output: all_diagnostics.join("\n"),
                            llm_content: None,
                            return_display: None,
                            error: None,
                            data: None,
                        })
                    }
                },
            )
            .await;

            match result {
                Ok(res) => res.map_err(|e| e as Box<dyn std::error::Error>),
                Err(e) => Err(Box::new(e) as Box<dyn std::error::Error>),
            }
        })
    }
}
