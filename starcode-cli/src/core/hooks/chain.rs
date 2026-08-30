use super::types::{EnhancedHookType, HookDefinition, HookEvent, HookResult};
use serde_json::Value;
use std::process::Stdio;

pub struct HookChain {
    pub hooks: Vec<HookDefinition>,
}

impl HookChain {
    pub fn new(hooks: Vec<HookDefinition>) -> Self {
        Self { hooks }
    }

    pub async fn execute(
        &self,
        event: HookEvent,
        tool_name: &str,
        input: &Value,
    ) -> Vec<HookResult> {
        let mut results = Vec::new();

        for hook in &self.hooks {
            if hook.event != event {
                continue;
            }

            if let Some(ref matcher) = hook.matcher {
                if !Self::matches(tool_name, matcher) {
                    continue;
                }
            }

            let result = match hook.hook_type {
                EnhancedHookType::Shell => self.execute_shell(hook, input).await,
                EnhancedHookType::Http => self.execute_http(hook, input).await,
                EnhancedHookType::Agent => self.execute_agent(hook, input).await,
                EnhancedHookType::Function => {
                    HookResult::block("Function hooks must use FunctionHookRegistry")
                }
            };

            results.push(result);

            if hook.blocking {
                if let Some(last) = results.last() {
                    if last.is_blocking() {
                        break;
                    }
                }
            }
        }

        results
    }

    fn matches(tool_name: &str, pattern: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return tool_name.starts_with(prefix);
        }
        tool_name == pattern
    }

    async fn execute_shell(&self, hook: &HookDefinition, input: &Value) -> HookResult {
        let Some(ref command) = hook.command else {
            return HookResult::block("Shell hook missing command");
        };

        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = tokio::process::Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-lc").arg(command);
            c
        };

        cmd.env("STAR_HOOK_INPUT", input.to_string());
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let timeout = std::time::Duration::from_millis(hook.timeout_ms.max(1000));

        match tokio::time::timeout(timeout, cmd.output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() {
                    Self::parse_hook_output(&stdout)
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    HookResult::block(format!("Shell hook failed: {}", stderr))
                }
            }
            Ok(Err(e)) => HookResult::block(format!("Shell hook execution error: {}", e)),
            Err(_) => HookResult::block(format!(
                "Shell hook timed out after {}ms",
                hook.timeout_ms
            )),
        }
    }

    async fn execute_http(&self, hook: &HookDefinition, input: &Value) -> HookResult {
        let Some(ref url) = hook.url else {
            return HookResult::block("HTTP hook missing url");
        };

        let client = reqwest::Client::new();
        let timeout = std::time::Duration::from_millis(hook.timeout_ms.max(1000));

        match tokio::time::timeout(
            timeout,
            client.post(url).json(input).send(),
        )
        .await
        {
            Ok(Ok(resp)) => {
                if resp.status().is_success() {
                    match resp.json::<Value>().await {
                        Ok(body) => Self::parse_hook_json_output(&body),
                        Err(e) => HookResult::block(format!("HTTP hook JSON parse error: {}", e)),
                    }
                } else {
                    HookResult::block(format!("HTTP hook returned status: {}", resp.status()))
                }
            }
            Ok(Err(e)) => HookResult::block(format!("HTTP hook request error: {}", e)),
            Err(_) => HookResult::block(format!(
                "HTTP hook timed out after {}ms",
                hook.timeout_ms
            )),
        }
    }

    async fn execute_agent(&self, hook: &HookDefinition, input: &Value) -> HookResult {
        let Some(ref prompt) = hook.prompt else {
            return HookResult::block("Agent hook missing prompt");
        };
        HookResult::allow().with_additional_context(format!(
            "Agent hook prompt: {} (input: {})",
            prompt, input
        ))
    }

    fn parse_hook_output(stdout: &str) -> HookResult {
        if let Ok(json) = serde_json::from_str::<Value>(stdout) {
            return Self::parse_hook_json_output(&json);
        }

        if stdout.is_empty() {
            return HookResult::allow();
        }

        HookResult::allow().with_additional_context(stdout.to_string())
    }

    fn parse_hook_json_output(json: &Value) -> HookResult {
        let decision = json.get("decision").and_then(|v| v.as_str());

        match decision {
            Some("deny") => HookResult::deny(),
            Some("block") => {
                let reason = json
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Blocked by hook");
                HookResult::block(reason)
            }
            Some("ask") => {
                let prompt = json
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Confirmation required");
                HookResult::ask(prompt)
            }
            _ => {
                let mut result = HookResult::allow();

                if let Some(updated) = json.get("updated_input") {
                    result = result.with_updated_input(updated.clone());
                }
                if let Some(ctx) = json.get("additional_context").and_then(|v| v.as_str()) {
                    result = result.with_additional_context(ctx);
                }
                if let Some(msg) = json.get("message").and_then(|v| v.as_str()) {
                    result = result.with_message(msg);
                }

                result
            }
        }
    }
}

 