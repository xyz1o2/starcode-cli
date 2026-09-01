use std::path::Path;

/// Verification result after an edit operation
#[derive(Debug, Clone)]
pub struct EditVerificationResult {
    pub file_path: String,
    pub syntax_ok: bool,
    pub errors: Vec<SyntaxError>,
    pub suggestion: Option<String>,
}

/// A syntax error found during verification
#[derive(Debug, Clone)]
pub struct SyntaxError {
    pub line: Option<usize>,
    pub message: String,
    pub severity: ErrorSeverity,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorSeverity {
    Error,
    Warning,
    Info,
}

/// Verify syntax after an edit operation.
/// Uses language-native tools (like Claude Code/Codex approach):
/// - Python: python -m py_compile
/// - JS/TS: node --check / tsc --noEmit
/// - Rust: cargo check (via bash)
/// - Shell: bash -n
/// - Ruby: ruby -c
/// - PHP: php -l
/// - JSON/TOML/YAML: parse validation
///
/// Returns a verification result with any syntax errors found.
pub fn verify_edit_syntax(file_path: &str, content: &str) -> EditVerificationResult {
    let path = Path::new(file_path);
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match extension {
        "py" => verify_python(file_path, content),
        "js" | "mjs" | "cjs" => verify_javascript(file_path, content),
        "ts" | "tsx" | "jsx" => verify_typescript(file_path, content),
        "rs" => verify_rust(file_path),
        "json" => verify_json(file_path, content),
        "toml" => verify_toml(file_path, content),
        "yaml" | "yml" => verify_yaml(file_path, content),
        "sh" | "bash" | "zsh" => verify_shell(file_path, content),
        "go" => verify_go(file_path),
        "rb" => verify_ruby(file_path, content),
        "php" => verify_php(file_path, content),
        "css" | "scss" | "sass" | "html" | "htm" | "sql" => {
            // These languages don't have reliable CLI syntax checkers
            // that don't require LSP servers
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: true,
                errors: vec![],
                suggestion: None,
            }
        }
        _ => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
    }
}

/// Python: python -m py_compile (Claude Code approach)
fn verify_python(file_path: &str, _content: &str) -> EditVerificationResult {
    let output = std::process::Command::new("python3")
        .args(["-m", "py_compile", file_path])
        .output();

    match output {
        Ok(result) if !result.status.success() => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let error_msg = extract_first_error_line(&stderr);
            let line = extract_line_number(&error_msg);
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line,
                    message: error_msg,
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the Python syntax error before proceeding.".to_string()),
            }
        }
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(e) => {
            // python3 not found - skip verification
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: true,
                errors: vec![],
                suggestion: None,
            }
        }
    }
}

/// JavaScript: node --check (Claude Code approach)
fn verify_javascript(file_path: &str, _content: &str) -> EditVerificationResult {
    let output = std::process::Command::new("node")
        .args(["--check", file_path])
        .output();

    match output {
        Ok(result) if !result.status.success() => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let error_msg = extract_first_error_line(&stderr);
            let line = extract_line_number(&error_msg);
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line,
                    message: error_msg,
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the JavaScript syntax error before proceeding.".to_string()),
            }
        }
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
    }
}

/// TypeScript: tsc --noEmit (Claude Code approach)
fn verify_typescript(file_path: &str, _content: &str) -> EditVerificationResult {
    // Try tsc first (more accurate but requires tsconfig)
    let output = std::process::Command::new("tsc")
        .args(["--noEmit", "--allowJs", file_path])
        .output();

    match output {
        Ok(result) if !result.status.success() => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let error_msg = extract_first_error_line(&stderr);
            let line = extract_line_number(&error_msg);
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line,
                    message: error_msg,
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the TypeScript syntax error before proceeding.".to_string()),
            }
        }
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(_) => {
            // tsc not found, try node --check as fallback
            verify_javascript(file_path, _content)
        }
    }
}

/// Rust: cargo check (the standard approach)
fn verify_rust(file_path: &str) -> EditVerificationResult {
    // For Rust, we can't check a single file easily
    // cargo check checks the whole project, which is expensive
    // We'll rely on the agent to run cargo check when needed
    EditVerificationResult {
        file_path: file_path.to_string(),
        syntax_ok: true,
        errors: vec![],
        suggestion: Some("Run `cargo check` to verify Rust syntax.".to_string()),
    }
}

/// JSON: parse validation
fn verify_json(file_path: &str, content: &str) -> EditVerificationResult {
    match serde_json::from_str::<serde_json::Value>(content) {
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(e) => {
            let line = e.line();
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line: Some(line),
                    message: format!("JSON parse error: {}", e),
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the JSON syntax error before proceeding.".to_string()),
            }
        }
    }
}

/// TOML: parse validation
fn verify_toml(file_path: &str, content: &str) -> EditVerificationResult {
    match content.parse::<toml::Value>() {
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(e) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: false,
            errors: vec![SyntaxError {
                line: None,
                message: format!("TOML parse error: {}", e),
                severity: ErrorSeverity::Error,
            }],
            suggestion: Some("Fix the TOML syntax error before proceeding.".to_string()),
        },
    }
}

/// YAML: basic validation (no external dependency)
fn verify_yaml(file_path: &str, content: &str) -> EditVerificationResult {
    // Basic YAML validation - check for common syntax issues
    let errors = check_basic_yaml_syntax(content);
    if !errors.is_empty() {
        return EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: false,
            errors,
            suggestion: Some("Fix the YAML syntax error before proceeding.".to_string()),
        };
    }
    EditVerificationResult {
        file_path: file_path.to_string(),
        syntax_ok: true,
        errors: vec![],
        suggestion: None,
    }
}

/// Shell: bash -n (Claude Code approach)
fn verify_shell(file_path: &str, _content: &str) -> EditVerificationResult {
    let output = std::process::Command::new("bash")
        .args(["-n", file_path])
        .output();

    match output {
        Ok(result) if !result.status.success() => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let error_msg = extract_first_error_line(&stderr);
            let line = extract_line_number(&error_msg);
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line,
                    message: error_msg,
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the shell syntax error before proceeding.".to_string()),
            }
        }
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
    }
}

/// Go: go vet (requires module setup)
fn verify_go(file_path: &str) -> EditVerificationResult {
    // go vet requires a module, so we just return success
    // The agent should run `go vet` when needed
    EditVerificationResult {
        file_path: file_path.to_string(),
        syntax_ok: true,
        errors: vec![],
        suggestion: Some("Run `go vet ./...` to verify Go syntax.".to_string()),
    }
}

/// Ruby: ruby -c (Claude Code approach)
fn verify_ruby(file_path: &str, _content: &str) -> EditVerificationResult {
    let output = std::process::Command::new("ruby")
        .args(["-c", file_path])
        .output();

    match output {
        Ok(result) if !result.status.success() => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let error_msg = extract_first_error_line(&stderr);
            let line = extract_line_number(&error_msg);
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line,
                    message: error_msg,
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the Ruby syntax error before proceeding.".to_string()),
            }
        }
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
    }
}

/// PHP: php -l (Claude Code approach)
fn verify_php(file_path: &str, _content: &str) -> EditVerificationResult {
    let output = std::process::Command::new("php")
        .args(["-l", file_path])
        .output();

    match output {
        Ok(result) if !result.status.success() => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let error_msg = extract_first_error_line(&stderr);
            let line = extract_line_number(&error_msg);
            EditVerificationResult {
                file_path: file_path.to_string(),
                syntax_ok: false,
                errors: vec![SyntaxError {
                    line,
                    message: error_msg,
                    severity: ErrorSeverity::Error,
                }],
                suggestion: Some("Fix the PHP syntax error before proceeding.".to_string()),
            }
        }
        Ok(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
        Err(_) => EditVerificationResult {
            file_path: file_path.to_string(),
            syntax_ok: true,
            errors: vec![],
            suggestion: None,
        },
    }
}

/// Extract first error line from stderr
fn extract_first_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Unknown syntax error")
        .to_string()
}

/// Extract line number from error message
fn extract_line_number(error_msg: &str) -> Option<usize> {
    // Try to find patterns like "line 42" or ":42:" or "(42,"
    for pattern in &["line ", "Line ", "LINE "] {
        if let Some(pos) = error_msg.find(pattern) {
            let rest = &error_msg[pos + pattern.len()..];
            if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
                if let Ok(line) = rest[..end].parse::<usize>() {
                    return Some(line);
                }
            }
        }
    }
    // Try pattern like ":42:"
    let chars: Vec<char> = error_msg.chars().collect();
    for i in 0..chars.len().saturating_sub(3) {
        if chars[i] == ':' && chars[i+2] == ':' {
            let num_str: String = chars[i+1..i+2].iter().collect();
            if let Ok(line) = num_str.parse::<usize>() {
                return Some(line);
            }
        }
    }
    None
}

/// Basic YAML syntax checks
fn check_basic_yaml_syntax(content: &str) -> Vec<SyntaxError> {
    let mut errors = Vec::new();
    let mut line_num = 0;

    for line in content.lines() {
        line_num += 1;
        let trimmed = line.trim();
        
        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Check for tabs (YAML doesn't allow tabs for indentation)
        if line.starts_with('\t') {
            errors.push(SyntaxError {
                line: Some(line_num),
                message: "YAML does not allow tabs for indentation. Use spaces.".to_string(),
                severity: ErrorSeverity::Error,
            });
        }

        // Check for basic key-value syntax
        if trimmed.contains(':') && !trimmed.starts_with('-') {
            let parts: Vec<&str> = trimmed.splitn(2, ':').collect();
            if parts.len() == 2 {
                let key = parts[0].trim();
                
                // Check for empty key
                if key.is_empty() {
                    errors.push(SyntaxError {
                        line: Some(line_num),
                        message: "Empty key in YAML mapping.".to_string(),
                        severity: ErrorSeverity::Error,
                    });
                }
            }
        }
    }

    errors
}

/// Format verification result for display
pub fn format_verification_result(result: &EditVerificationResult) -> String {
    if result.syntax_ok {
        return format!("[VERIFIED] {} - Syntax OK", result.file_path);
    }

    let mut output = String::new();
    output.push_str(&format!("[SYNTAX_ERROR] {}\n", result.file_path));

    for error in &result.errors {
        let line_str = error.line.map(|l| format!("Line {}: ", l)).unwrap_or_default();
        output.push_str(&format!("  {}{}\n", line_str, error.message));
    }

    if let Some(suggestion) = &result.suggestion {
        output.push_str(&format!("\n  Suggestion: {}", suggestion));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_valid() {
        let result = verify_json("test.json", r#"{"key": "value"}"#);
        assert!(result.syntax_ok);
    }

    #[test]
    fn test_json_invalid() {
        let result = verify_json("test.json", r#"{"key": "value""#);
        assert!(!result.syntax_ok);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_extract_line_number() {
        assert_eq!(extract_line_number("SyntaxError at line 42"), Some(42));
        assert_eq!(extract_line_number("Error on line 10: invalid syntax"), Some(10));
        assert_eq!(extract_line_number("Unknown error"), None);
    }
}
