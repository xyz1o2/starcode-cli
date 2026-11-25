use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolError, ToolInvocation, ToolLocation,
    ToolResult as CoreToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Clone)]
pub struct NextEditTool {
    base_dir: PathBuf,
}

impl NextEditTool {
    pub fn new() -> Self {
        Self::with_base_dir(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NextEditParams {
    pub file_path: String,
    pub symbol_name: String,
    pub symbol_type: Option<String>, // "function", "struct", "enum", "method"
}

pub struct NextEditInvocation {
    params: NextEditParams,
    base_dir: PathBuf,
}

impl ToolInvocation for NextEditInvocation {
    fn get_description(&self) -> String {
        format!(
            "Analyzing impact for symbol '{}' in {}",
            self.params.symbol_name, self.params.file_path
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
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let file_path = self.params.file_path.clone();
        let symbol_name = self.params.symbol_name.clone();
        let base_dir = self.base_dir.clone();

        Box::pin(async move {
            let result = analyze_impact(&file_path, &symbol_name, &base_dir).await;

            match result {
                Ok(output) => Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output,
                    error: None,
                    data: None,
                }),
                Err(e) => Ok(CoreToolResult {
                    llm_content: None,
                    return_display: None,
                    output: String::new(),
                    error: Some(ToolError {
                        error_type: "execution_error".to_string(),
                        message: e.to_string(),
                    }),
                    data: None,
                }),
            }
        })
    }
}

impl BaseDeclarativeTool for NextEditTool {
    fn name(&self) -> &str {
        "NextEdit"
    }

    fn display_name(&self) -> &str {
        "Next Edit Analysis"
    }

    fn description(&self) -> &str {
        "Analyze the impact of changing a symbol (function, struct, etc.) by finding all references and dependencies."
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file containing the symbol definition"
                },
                "symbol_name": {
                    "type": "string",
                    "description": "Name of the symbol (function, struct, etc.) to analyze"
                },
                "symbol_type": {
                    "type": "string",
                    "enum": ["function", "struct", "enum", "method"],
                    "description": "Optional type of the symbol to refine search"
                }
            },
            "required": ["file_path", "symbol_name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: NextEditParams = serde_json::from_value(params)?;
        Ok(Box::new(NextEditInvocation {
            params,
            base_dir: self.base_dir.clone(),
        }))
    }

    // 纯分析：ripgrep/内嵌搜索找引用 + 读文件，不改任何磁盘状态。
    fn is_read_only(&self) -> bool {
        true
    }
}

/// 解析 `rg --json` 的 match 事件，避免用冒号拆分路径。
fn parse_rg_matches(output: &[u8]) -> Vec<(String, u64)> {
    let text = String::from_utf8_lossy(output);
    text.lines()
        .filter_map(|line| {
            let event = serde_json::from_str::<serde_json::Value>(line).ok()?;
            if event.get("type").and_then(serde_json::Value::as_str) != Some("match") {
                return None;
            }
            let data = event.get("data")?;
            let path = data
                .get("path")
                .and_then(|path| path.get("text"))
                .and_then(serde_json::Value::as_str)?;
            let line_number = data
                .get("line_number")
                .and_then(serde_json::Value::as_u64)?;
            Some((path.to_string(), line_number))
        })
        .collect()
}

fn normalize_search_path(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

fn ripgrep_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install with `brew install ripgrep`"
    } else if cfg!(target_os = "windows") {
        "Install with `winget install BurntSushi.ripgrep.MSVC` or `choco install ripgrep`"
    } else {
        "Install with `sudo apt install ripgrep` (Debian/Ubuntu), `sudo dnf install ripgrep` (Fedora), or `sudo pacman -S ripgrep` (Arch)"
    }
}

async fn analyze_impact(
    file_path: &str,
    symbol_name: &str,
    base_dir: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    // 1. Prefer ripgrep for fast workspace search. Unlike `git grep`, it also
    // searches files that have not been staged or tracked yet.
    let rg_output = Command::new("rg")
        .args(["--json", "--fixed-strings", "-e", symbol_name, "."])
        .current_dir(base_dir)
        .output()
        .await;

    let mut usages: HashMap<String, Vec<u64>> = HashMap::new();
    let mut files_to_parse: HashSet<String> = HashSet::new();
    let mut fallback_notice = None;
    let use_native_fallback = match rg_output {
        Ok(output) if output.status.success() => {
            for (path, line_num) in parse_rg_matches(&output.stdout) {
                let path = normalize_search_path(&path);
                usages.entry(path.clone()).or_default().push(line_num);
                if path.ends_with(".rs") {
                    files_to_parse.insert(path);
                }
            }
            false
        }
        // `rg` uses exit code 1 for a valid search with no matches. This is a
        // real empty result, not a missing executable, so do not run fallback.
        Ok(output) if output.status.code() == Some(1) => false,
        Ok(output) => {
            fallback_notice = Some(format!(
                "ripgrep (`rg`) could not complete the search ({}); I used StarCode's built-in search fallback.",
                output.status
            ));
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fallback_notice = Some(
                format!(
                    "ripgrep (`rg`) is not installed. I used StarCode's built-in search fallback. Would you like to install ripgrep? {}. Do not install software without confirmation.",
                    ripgrep_install_hint()
                ),
            );
            true
        }
        Err(error) => {
            fallback_notice = Some(format!(
                "ripgrep (`rg`) could not be started ({}); I used StarCode's built-in search fallback.",
                error
            ));
            true
        }
    };

    if use_native_fallback {
        let fallback_root = base_dir.to_path_buf();
        let fallback_symbol = symbol_name.to_string();
        let native = tokio::task::spawn_blocking(move || {
            let config = crate::core::tools::ripgrep::RipgrepConfig {
                case_sensitive: true,
                whole_word: false,
                regex: false,
                max_results: Some(10_000),
                file_types: None,
                exclude_patterns: None,
                include_patterns: None,
                include_base: None,
                include_hidden: true,
                max_file_size: Some(2 * 1024 * 1024),
            };
            crate::core::tools::ripgrep::search_with_ripgrep_stats(
                &fallback_symbol,
                &fallback_root.to_string_lossy(),
                config,
            )
            .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?
        .map_err(std::io::Error::other)?;

        for matched in native.0 {
            let path = Path::new(&matched.file)
                .strip_prefix(base_dir)
                .unwrap_or(Path::new(&matched.file))
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            let line_num = matched.line.unwrap_or_default() as u64;
            usages.entry(path.clone()).or_default().push(line_num);
            if path.ends_with(".rs") {
                files_to_parse.insert(path);
            }
        }
    }

    // 2. Report generation
    let mut report = String::new();
    if let Some(notice) = fallback_notice {
        report.push_str(&notice);
        report.push_str("\n\n");
    }
    report.push_str(&format!(
        "Impact Analysis for '{}' (defined in {}):\n\n",
        symbol_name, file_path
    ));

    let total_files = usages.len();
    let total_refs = usages.values().map(|v| v.len()).sum::<usize>();

    report.push_str(&format!(
        "Found {} references in {} files.\n\n",
        total_refs, total_files
    ));

    for (file, lines) in usages {
        report.push_str(&format!("File: {}\n", file));
        let lines_str: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        report.push_str(&format!("  Lines: {}\n", lines_str.join(", ")));

        // Context snippet for first match
        if let Some(first_line) = lines.first() {
            // Read file line
            if let Ok(content) = tokio::fs::read_to_string(base_dir.join(&file)).await {
                let content_lines: Vec<&str> = content.lines().collect();
                if let Some(line_content) =
                    content_lines.get((*first_line as usize).saturating_sub(1))
                {
                    report.push_str(&format!("  Snippet: {}\n", line_content.trim()));
                }
            }
        }
        report.push('\n');
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn rg_json_parser_preserves_colons_in_paths() {
        let output = json!({
            "type": "match",
            "data": {
                "path": {"text": "src/a:b.rs"},
                "line_number": 9
            }
        })
        .to_string();

        assert_eq!(
            parse_rg_matches(output.as_bytes()),
            vec![("src/a:b.rs".to_string(), 9)]
        );
    }

    #[tokio::test]
    async fn impact_search_and_snippets_use_the_bound_base_dir() {
        let root = tempfile::tempdir().expect("create temporary root");
        let parent = root.path().join("parent");
        let child = root.path().join("child");
        std::fs::create_dir_all(&parent).expect("create parent directory");
        std::fs::create_dir_all(&child).expect("create child directory");
        std::fs::write(
            parent.join("same.rs"),
            "fn shared_symbol() { /* parent context */ }\n",
        )
        .expect("write parent source");
        std::fs::write(
            child.join("same.rs"),
            "fn shared_symbol() { /* child context */ }\n",
        )
        .expect("write child source");

        let tool = NextEditTool::with_base_dir(PathBuf::from(child));
        let invocation = tool
            .create_invocation(json!({
                "file_path": "same.rs",
                "symbol_name": "shared_symbol",
            }))
            .expect("create NextEdit invocation");
        let result = invocation
            .execute(None, None)
            .await
            .expect("execute NextEdit invocation");

        assert!(result.error.is_none());
        assert!(result.output.contains("child context"));
        assert!(!result.output.contains("parent context"));
    }
}
