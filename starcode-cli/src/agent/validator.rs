use crate::types::ToolResult;
use serde_json::Value;
use std::collections::HashMap;

pub struct Validator;

impl Validator {
    fn repair_json_string(input: &str) -> String {
        // 1. Remove markdown code blocks if present
        let input = input.trim();
        let input = if input.starts_with("```json") {
            input.strip_prefix("```json").unwrap_or(input)
        } else if input.starts_with("```") {
            input.strip_prefix("```").unwrap_or(input)
        } else {
            input
        };
        let input = if input.ends_with("```") {
            input.strip_suffix("```").unwrap_or(input)
        } else {
            input
        };

        let input = input.trim();

        // 2. Attempt to fix unescaped backslashes (common in Windows paths from LLMs)
        let mut output = String::with_capacity(input.len() + 32);
        let mut chars = input.chars();

        while let Some(c) = chars.next() {
            if c == '\\' {
                // Look ahead
                let next = chars.clone().next(); // Clone iterator to peek
                if let Some(n) = next {
                    match n {
                        // Valid escape sequences in JSON
                        '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u' => {
                            output.push('\\');
                            output.push(n);
                            chars.next(); // Consume the valid escape char
                        }
                        _ => {
                            // Invalid escape sequence (e.g. \a, \U, \P)
                            // Assume it's a Windows path separator and escape the backslash
                            output.push('\\');
                            output.push('\\');
                            // Do NOT consume 'n', it is part of the path name
                        }
                    }
                } else {
                    // Trailing backslash at end of string
                    output.push('\\');
                    output.push('\\');
                }
            } else {
                output.push(c);
            }
        }

        output
    }

    fn repair_truncated_json(input: &str) -> String {
        #[derive(Debug, Clone, Copy, PartialEq)]
        enum Scope {
            Object, // {
            Array,  // [
        }

        #[derive(Debug, Clone, Copy, PartialEq)]
        enum Expectation {
            Key,
            Colon,
            Value,
            Comma,
        }

        let mut stack: Vec<(Scope, Expectation)> = Vec::new();
        let mut in_string = false;
        let mut is_escaped = false;

        for c in input.chars() {
            if is_escaped {
                is_escaped = false;
                continue;
            }

            if c == '\\' {
                is_escaped = true;
                continue;
            }

            if in_string {
                if c == '"' {
                    in_string = false;
                    // String finished. Update expectation.
                    if let Some((scope, expect)) = stack.last_mut() {
                        match (scope, *expect) {
                            (Scope::Object, Expectation::Key) => *expect = Expectation::Colon,
                            (Scope::Object, Expectation::Value) => *expect = Expectation::Comma,
                            (Scope::Array, Expectation::Value) => *expect = Expectation::Comma,
                            _ => {}
                        }
                    }
                }
                continue;
            }

            match c {
                '"' => in_string = true,
                '{' => {
                    // Update current expectation before pushing new scope
                    if let Some((scope, expect)) = stack.last_mut() {
                        match (*scope, *expect) {
                            (Scope::Object, Expectation::Value) => *expect = Expectation::Comma,
                            (Scope::Array, Expectation::Value) => *expect = Expectation::Comma,
                            _ => {}
                        }
                    }
                    stack.push((Scope::Object, Expectation::Key));
                }
                '[' => {
                    if let Some((scope, expect)) = stack.last_mut() {
                        match (*scope, *expect) {
                            (Scope::Object, Expectation::Value) => *expect = Expectation::Comma,
                            (Scope::Array, Expectation::Value) => *expect = Expectation::Comma,
                            _ => {}
                        }
                    }
                    stack.push((Scope::Array, Expectation::Value));
                }
                ':' => {
                    if let Some((Scope::Object, expect)) = stack.last_mut() {
                        if *expect == Expectation::Colon {
                            *expect = Expectation::Value;
                        }
                    }
                }
                ',' => {
                    if let Some((scope, expect)) = stack.last_mut() {
                        match scope {
                            Scope::Object => *expect = Expectation::Key,
                            Scope::Array => *expect = Expectation::Value,
                        }
                    }
                }
                '}' => {
                    if let Some((Scope::Object, _)) = stack.last() {
                        stack.pop();
                        // Closing an object is like finishing a value in the parent
                        if let Some((scope, expect)) = stack.last_mut() {
                            match (*scope, *expect) {
                                (Scope::Object, Expectation::Value) => *expect = Expectation::Comma,
                                (Scope::Array, Expectation::Value) => *expect = Expectation::Comma,
                                _ => {}
                            }
                        }
                    }
                }
                ']' => {
                    if let Some((Scope::Array, _)) = stack.last() {
                        stack.pop();
                        if let Some((scope, expect)) = stack.last_mut() {
                            match (*scope, *expect) {
                                (Scope::Object, Expectation::Value) => *expect = Expectation::Comma,
                                (Scope::Array, Expectation::Value) => *expect = Expectation::Comma,
                                _ => {}
                            }
                        }
                    }
                }
                // Ignore whitespace
                _ => {}
            }
        }

        let mut output = input.to_string();

        // 1. Close string if open
        if in_string {
            output.push('"');
            // String finished. Update expectation logic effectively
            if let Some((scope, expect)) = stack.last_mut() {
                match (scope, *expect) {
                    (Scope::Object, Expectation::Key) => *expect = Expectation::Colon,
                    (Scope::Object, Expectation::Value) => *expect = Expectation::Comma,
                    (Scope::Array, Expectation::Value) => *expect = Expectation::Comma,
                    _ => {}
                }
            }
        }

        // 2. Unwind stack
        while let Some((scope, expect)) = stack.pop() {
            match scope {
                Scope::Object => {
                    match expect {
                        Expectation::Key => {
                            // Trailing comma or just {
                            // Check if output ends with comma, remove it
                            if let Some(idx) = output.rfind(',') {
                                let suffix = &output[idx + 1..];
                                if suffix.trim().is_empty() {
                                    output.truncate(idx);
                                }
                            }
                            output.push('}');
                        }
                        Expectation::Colon => {
                            // We have {"key" -> need : null }
                            output.push_str(": null}");
                        }
                        Expectation::Value => {
                            // We have {"key": -> need null }
                            output.push_str(" null}");
                        }
                        Expectation::Comma => {
                            // We have {"k":"v" -> need }
                            output.push('}');
                        }
                    }
                }
                Scope::Array => {
                    match expect {
                        Expectation::Value => {
                            // [ or [v,
                            if let Some(idx) = output.rfind(',') {
                                let suffix = &output[idx + 1..];
                                if suffix.trim().is_empty() {
                                    output.truncate(idx);
                                }
                            }
                            output.push(']');
                        }
                        Expectation::Comma => {
                            // [v
                            output.push(']');
                        }
                        _ => output.push(']'), // Should not happen
                    }
                }
            }
        }

        output
    }

    pub fn parse_args(arguments: &str) -> Result<HashMap<String, Value>, ToolResult> {
        // Strategy: Try standard parsing first. If it fails, try to repair the JSON string
        // (handling markdown blocks and unescaped Windows paths) and parse again.
        match serde_json::from_str::<HashMap<String, Value>>(arguments) {
            Ok(v) => Ok(v),
            Err(original_err) => {
                let repaired = Self::repair_json_string(arguments);

                // Try parsing the repaired string directly
                if let Ok(v) = serde_json::from_str::<HashMap<String, Value>>(&repaired) {
                    return Ok(v);
                }

                // If it looks like an EOF error (truncated JSON), try to auto-close it
                if original_err.is_eof() || original_err.to_string().contains("EOF") {
                    let candidate = Self::repair_truncated_json(&repaired);
                    if let Ok(v) = serde_json::from_str::<HashMap<String, Value>>(&candidate) {
                        return Ok(v);
                    }
                }

                serde_json::from_str::<HashMap<String, Value>>(&repaired).map_err(|e| ToolResult {
                    success: false,
                    output: None,
                    error: Some(format!(
                        "Invalid tool arguments JSON: {}. \nOriginal Error: {}. \nRepaired JSON: {}", 
                        e, original_err, repaired
                    )),
                    data: None,
                })
            }
        }
    }

    pub fn require_str(args: &HashMap<String, Value>, key: &str) -> Result<String, ToolResult> {
        args.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| ToolResult {
                success: false,
                output: None,
                error: Some(format!("Missing or invalid '{}' argument", key)),
                data: None,
            })
    }

    pub fn opt_u64(args: &HashMap<String, Value>, key: &str) -> Option<u64> {
        args.get(key).and_then(|v| v.as_u64())
    }

    pub fn opt_bool(args: &HashMap<String, Value>, key: &str) -> Option<bool> {
        args.get(key).and_then(|v| v.as_bool())
    }

    pub fn value(args: &HashMap<String, Value>, key: &str) -> Option<Value> {
        args.get(key).cloned()
    }
}
