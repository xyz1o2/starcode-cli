use crate::core::tools::constants::ToolName;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::paths::{make_relative, shorten_path};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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
        ToolName::GetDiagnostics.as_str()
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
                "uri": {"type": "string", "description": "Optional file path to filter diagnostics for."}
            }
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(GetDiagnosticsInvocation {
            config: self.config.clone(),
            params: serde_json::from_value(params)?,
        }))
    }
}

pub struct GetDiagnosticsInvocation {
    config: Arc<crate::core::config::Config>,
    params: GetDiagnosticsParams,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Checker {
    Rust,
    Eslint,
    Pylint,
}

impl Checker {
    fn available(self, root: &Path) -> bool {
        match self {
            Self::Rust => which::which("cargo").is_ok(),
            Self::Eslint => {
                let local = root.join("node_modules/.bin");
                (local.join("eslint").is_file() || local.join("eslint.cmd").is_file())
                    && which::which(if cfg!(windows) { "npx.cmd" } else { "npx" }).is_ok()
            }
            Self::Pylint => which::which("pylint").is_ok(),
        }
    }

    fn supports(self, path: &Path) -> bool {
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        let extension = path
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        match self {
            Self::Rust => extension == "rs" || matches!(name, "Cargo.toml" | "Cargo.lock"),
            Self::Eslint => {
                matches!(
                    extension,
                    "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts"
                ) || name == "package.json"
                    || name.starts_with("eslint.config.")
                    || name.starts_with(".eslintrc")
            }
            Self::Pylint => {
                extension == "py"
                    || matches!(name, "pyproject.toml" | "requirements.txt" | ".pylintrc")
            }
        }
    }

    fn command(self, root: &Path) -> Command {
        let mut command = match self {
            Self::Rust => {
                let mut command = Command::new("cargo");
                command.args(["check", "--message-format=json"]);
                command
            }
            Self::Eslint if cfg!(windows) => {
                let mut command = Command::new("cmd");
                command.args([
                    "/C",
                    "npx",
                    "--no-install",
                    "eslint",
                    ".",
                    "--format",
                    "json",
                ]);
                command
            }
            Self::Eslint => {
                let mut command = Command::new("npx");
                command.args(["--no-install", "eslint", ".", "--format", "json"]);
                command
            }
            Self::Pylint => {
                let mut command = Command::new("pylint");
                command.args([".", "--output-format=json", "--recursive=y"]);
                command
            }
        };
        command.current_dir(root);
        command
    }
}

fn detected_checkers(root: &Path) -> Vec<Checker> {
    let mut checkers = Vec::new();
    if root.join("Cargo.toml").is_file() {
        checkers.push(Checker::Rust);
    }
    if root.join("package.json").is_file() {
        checkers.push(Checker::Eslint);
    }
    if root.join("requirements.txt").is_file() || root.join("pyproject.toml").is_file() {
        checkers.push(Checker::Pylint);
    }
    checkers
}

/// 能力探测只读文件与 PATH，不运行检查器；纯资源编辑不触发全项目检查。
pub(crate) fn supports_automatic_check(root: &Path, edited_paths: &[PathBuf]) -> bool {
    detected_checkers(root).into_iter().any(|checker| {
        checker.available(root) && edited_paths.iter().any(|path| checker.supports(path))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    Passed,
    Issues,
    Failed,
    Unavailable,
}

#[derive(Debug, Serialize)]
struct CheckReport {
    checker: Checker,
    status: CheckStatus,
    diagnostics: Vec<String>,
    detail: String,
}

#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<DiagnosticMessage>,
    success: Option<bool>,
}

#[derive(Deserialize)]
struct DiagnosticMessage {
    message: String,
    level: String,
    spans: Vec<DiagnosticSpan>,
}

#[derive(Deserialize)]
struct DiagnosticSpan {
    file_name: String,
    line_start: u64,
    column_start: u64,
    is_primary: bool,
}

#[derive(Deserialize)]
struct EslintFile {
    #[serde(rename = "filePath")]
    file_path: String,
    messages: Vec<EslintMessage>,
}

#[derive(Deserialize)]
struct EslintMessage {
    #[serde(rename = "ruleId")]
    rule_id: Option<String>,
    severity: u8,
    message: String,
    #[serde(default)]
    line: u64,
    #[serde(default)]
    column: u64,
}

#[derive(Deserialize)]
struct PylintMessage {
    #[serde(rename = "type")]
    msg_type: String,
    path: String,
    line: u64,
    column: u64,
    message: String,
    symbol: String,
}

fn parse_checker_output(
    checker: Checker,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    filter: Option<&str>,
    root: &Path,
) -> CheckReport {
    let mut report = CheckReport {
        checker,
        status: CheckStatus::Failed,
        diagnostics: Vec::new(),
        detail: String::new(),
    };
    let mut issue_count = 0;
    let parsed = (|| -> Result<bool, serde_json::Error> {
        match checker {
            Checker::Rust => {
                let mut finished = None;
                for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
                    let msg: CargoMessage = serde_json::from_str(line)?;
                    if msg.reason == "build-finished" {
                        finished = msg.success;
                    }
                    if msg.reason != "compiler-message" {
                        continue;
                    }
                    if let Some(dm) = msg.message {
                        if !matches!(dm.level.as_str(), "error" | "warning") {
                            continue;
                        }
                        issue_count += 1;
                        if filter.is_some_and(|filter| {
                            !dm.spans.iter().any(|span| {
                                span.file_name.contains(filter) || filter.contains(&span.file_name)
                            })
                        }) {
                            continue;
                        }
                        let location = dm
                            .spans
                            .iter()
                            .find(|span| span.is_primary)
                            .or(dm.spans.first())
                            .map(|span| {
                                format!(
                                    "{}:{}:{}",
                                    span.file_name, span.line_start, span.column_start
                                )
                            })
                            .unwrap_or_else(|| "unknown".into());
                        report.diagnostics.push(format!(
                            "[RUST] [{}] {} - {}",
                            dm.level.to_uppercase(),
                            location,
                            dm.message
                        ));
                    }
                }
                Ok((exit_code == Some(0) && finished == Some(true))
                    || (exit_code.is_some_and(|code| code != 0)
                        && issue_count > 0
                        && finished == Some(false)))
            }
            Checker::Eslint => {
                let files: Vec<EslintFile> = serde_json::from_str(stdout)?;
                for file in files {
                    for msg in file.messages.into_iter().filter(|msg| msg.severity > 0) {
                        issue_count += 1;
                        if filter.is_some_and(|filter| !file.file_path.contains(filter)) {
                            continue;
                        }
                        let path = make_relative(Path::new(&file.file_path), root);
                        let severity = if msg.severity == 2 {
                            "ERROR"
                        } else {
                            "WARNING"
                        };
                        report.diagnostics.push(format!(
                            "[ESLINT] [{severity}] {}:{}:{} - {} ({})",
                            path.display(),
                            msg.line,
                            msg.column,
                            msg.message,
                            msg.rule_id.unwrap_or_default()
                        ));
                    }
                }
                Ok(exit_code == Some(0) || (exit_code == Some(1) && issue_count > 0))
            }
            Checker::Pylint => {
                let messages: Vec<PylintMessage> = serde_json::from_str(stdout)?;
                issue_count = messages.len();
                for msg in messages {
                    if filter.is_some_and(|filter| !msg.path.contains(filter)) {
                        continue;
                    }
                    report.diagnostics.push(format!(
                        "[PYLINT] [{}] {}:{}:{} - {} ({})",
                        msg.msg_type.to_uppercase(),
                        msg.path,
                        msg.line,
                        msg.column,
                        msg.message,
                        msg.symbol
                    ));
                }
                Ok(exit_code == Some(0)
                    || (exit_code.is_some_and(|code| (1..32).contains(&code)) && issue_count > 0))
            }
        }
    })();
    match parsed {
        Ok(true) => {
            report.status = if issue_count > 0 {
                CheckStatus::Issues
            } else {
                CheckStatus::Passed
            };
            report.detail = if issue_count > 0 {
                format!("{issue_count} diagnostic(s) reported before filtering.")
            } else {
                "Check completed successfully with no diagnostics.".into()
            };
        }
        result => {
            let reason = match result {
                Err(error) => format!("Could not parse checker output: {error}"),
                _ => "Checker did not complete successfully.".into(),
            };
            let stderr: String = stderr.chars().take(2000).collect();
            report.detail = format!("{reason} Exit code: {exit_code:?}. {stderr}");
        }
    }
    report
}

fn run_diagnostics_with(
    root: &Path,
    filter: Option<&str>,
    mut run: impl FnMut(Checker) -> std::io::Result<Output>,
) -> ToolResult {
    let reports = detected_checkers(root)
        .into_iter()
        .map(|checker| {
            if !checker.available(root) {
                return CheckReport {
                    checker,
                    status: CheckStatus::Unavailable,
                    diagnostics: Vec::new(),
                    detail: "Checker is not installed locally; no check was run.".into(),
                };
            }
            match run(checker) {
                Ok(output) => parse_checker_output(
                    checker,
                    output.status.code(),
                    &String::from_utf8_lossy(&output.stdout),
                    &String::from_utf8_lossy(&output.stderr),
                    filter,
                    root,
                ),
                Err(error) => CheckReport {
                    checker,
                    status: CheckStatus::Failed,
                    diagnostics: Vec::new(),
                    detail: format!("Could not execute checker: {error}"),
                },
            }
        })
        .collect::<Vec<_>>();
    let status = if reports.is_empty()
        || reports
            .iter()
            .any(|report| report.status == CheckStatus::Unavailable)
    {
        CheckStatus::Unavailable
    } else if reports
        .iter()
        .any(|report| report.status == CheckStatus::Failed)
    {
        CheckStatus::Failed
    } else if reports
        .iter()
        .any(|report| report.status == CheckStatus::Issues)
    {
        CheckStatus::Issues
    } else {
        CheckStatus::Passed
    };
    let output = if reports.is_empty() {
        "Verification was not run: no supported project checker was detected.".to_string()
    } else {
        reports
            .iter()
            .map(|report| {
                let mut text = format!(
                    "{:?}: {:?}. {}",
                    report.checker, report.status, report.detail
                );
                for diagnostic in &report.diagnostics {
                    text.push('\n');
                    text.push_str(diagnostic);
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    ToolResult {
        output,
        llm_content: None,
        return_display: None,
        error: matches!(status, CheckStatus::Failed | CheckStatus::Unavailable).then(|| ToolError {
            error_type: "verification_incomplete".into(),
            message: "Verification did not complete for all detected checkers; do not claim a clean build.".into(),
        }),
        data: Some(json!({"verification_status":status,"checks":reports})),
    }
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
        self.params
            .uri
            .as_ref()
            .map(|uri| ToolLocation {
                path: PathBuf::from(uri),
                location_type: crate::core::tools::tools::LocationType::Read,
            })
            .into_iter()
            .collect()
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
        let root = self.config.target_dir().to_path_buf();
        let filter = self.params.uri.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                run_diagnostics_with(&root, filter.as_deref(), |checker| {
                    checker.command(&root).output()
                })
            })
            .await
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_svg_does_not_trigger_a_project_cargo_check() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Cargo.toml"), "[package]\n").unwrap();
        assert!(!supports_automatic_check(root.path(), &["logo.svg".into()]));
        assert!(!supports_automatic_check(root.path(), &[]));
    }

    #[test]
    fn no_checker_is_unverified_not_a_clean_build() {
        let root = tempfile::tempdir().unwrap();
        let result = run_diagnostics_with(root.path(), None, |_| panic!("no checker should run"));
        assert_eq!(
            result.data.as_ref().unwrap()["verification_status"],
            "unavailable"
        );
        assert!(result.error.is_some());
        assert!(!result.output.contains("clean"));
    }

    #[test]
    fn fake_checker_results_distinguish_clean_issues_execution_and_parse_failures() {
        let clean = parse_checker_output(
            Checker::Rust,
            Some(0),
            "{\"reason\":\"build-finished\",\"success\":true}\n",
            "",
            None,
            Path::new("/fixture"),
        );
        assert_eq!(clean.status, CheckStatus::Passed);
        let failed = parse_checker_output(
            Checker::Rust,
            Some(101),
            "",
            "failed to fetch dependency",
            None,
            Path::new("/fixture"),
        );
        assert_eq!(failed.status, CheckStatus::Failed);
        let malformed = parse_checker_output(
            Checker::Eslint,
            Some(0),
            "not json",
            "",
            None,
            Path::new("/fixture"),
        );
        assert_eq!(malformed.status, CheckStatus::Failed);
        let empty = parse_checker_output(
            Checker::Eslint,
            Some(0),
            "[]",
            "",
            None,
            Path::new("/fixture"),
        );
        assert_eq!(empty.status, CheckStatus::Passed);
        let issue = parse_checker_output(
            Checker::Pylint,
            Some(2),
            r#"[{"type":"error","path":"a.py","line":1,"column":0,"message":"undefined name","symbol":"undefined-variable"}]"#,
            "",
            None,
            Path::new("/fixture"),
        );
        assert_eq!(issue.status, CheckStatus::Issues);
        let filtered = parse_checker_output(
            Checker::Pylint,
            Some(2),
            r#"[{"type":"error","path":"a.py","line":1,"column":0,"message":"undefined name","symbol":"undefined-variable"}]"#,
            "",
            Some("b.py"),
            Path::new("/fixture"),
        );
        assert_eq!(
            filtered.status,
            CheckStatus::Issues,
            "filtering diagnostics must not turn a failed check into clean"
        );
    }
}
