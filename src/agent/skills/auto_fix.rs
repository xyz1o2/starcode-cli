use crate::agent::skills::{SubTask, SubTaskResult};
use crate::agent::StarAgent;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    pub output: String,
    pub error: Option<String>,
    pub failed_tests: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AutoFixConfig {
    pub max_attempts: usize,
    pub test_command: String,
    pub timeout_secs: u64,
}

use crate::core::config::Config;
use crate::llm::client::StarClient;

pub struct AutoFixAgent {
    client: StarClient,
    config: Arc<Config>,
}

impl AutoFixAgent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    pub async fn run_loop(
        &self,
        config: AutoFixConfig,
        initial_context: &str,
    ) -> Result<TestResult, Box<dyn std::error::Error>> {
        let mut attempt = 0;
        let mut context = initial_context.to_string();

        // Create a fresh agent for the loop
        // We need to reconstruct ToolRegistry or reuse one?
        // Ideally we reuse the config's tool registry if it exists.
        // But StarAgent::new takes explicit params.

        let mut agent = StarAgent::new(
            &self.client.api_key,
            Some(self.client.model.clone()),
            self.client.base_url.clone(),
            Some(self.config.max_session_turns() as u32),
            Some(self.client.is_openai_compatible),
            Some(self.config.clone()),
        )
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        loop {
            // 1. Run tests
            let result = self.run_tests(&config).await?;
            if result.success {
                println!("✅ Tests passed successfully!");
                return Ok(result);
            }

            attempt += 1;
            if attempt >= config.max_attempts {
                println!("❌ Max attempts reached. Tests failed.");
                return Ok(result);
            }

            println!(
                "🔄 Attempt {}/{}: Tests failed. Analyzing and fixing...",
                attempt, config.max_attempts
            );

            // 2. Analyze failure and prompt agent
            let prompt = self.construct_fix_prompt(&result, &context);

            // 3. Agent attempts fix
            // Note: In a real implementation, we would handle the conversation history properly
            // Here we're simplifying by sending a new message
            let _response = agent
                .process_user_message(&prompt)
                .await
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

            // Update context if needed
            context = format!(
                "Previous attempt failed with: {}\n\nTrying again...",
                result.output
            );
        }
    }

    async fn run_tests(
        &self,
        config: &AutoFixConfig,
    ) -> Result<TestResult, Box<dyn std::error::Error>> {
        println!("🏃 Running tests: {}", config.test_command);

        // Simple command parsing (split by space)
        // In production, use shell_words or similar
        let parts: Vec<&str> = config.test_command.split_whitespace().collect();
        if parts.is_empty() {
            return Err("Empty test command".into());
        }

        let output = Command::new(parts[0]).args(&parts[1..]).output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let combined_output = format!("{}\n{}", stdout, stderr);

        // Simple heuristic for failed tests (can be improved based on test runner)
        // For pytest/cargo test, we usually look for "FAIL" or "failed"
        let failed_tests = self.parse_failed_tests(&combined_output);

        Ok(TestResult {
            success: output.status.success(),
            output: combined_output,
            error: if !output.status.success() {
                Some(stderr)
            } else {
                None
            },
            failed_tests,
        })
    }

    fn parse_failed_tests(&self, output: &str) -> Vec<String> {
        let mut failed = Vec::new();
        // Heuristic for Python unittest/pytest
        for line in output.lines() {
            if line.starts_with("FAIL:") || line.contains("FAILED") {
                failed.push(line.to_string());
            }
        }
        failed
    }

    fn construct_fix_prompt(&self, result: &TestResult, context: &str) -> String {
        format!(
            "Task: Fix the code failures based on the test output.

<context>
{context}
</context>

<test_results>
Status: Failed ❌
Failed Tests:
{failed_tests}

Full Output (Truncated):
{full_output}
</test_results>

<instructions>
1.  **🔍 ANALYZE**: Read the test output. Which file/function is broken?
2.  **🧠 THINK**: Why did it fail?
    *   Hypothesis 1: Logic error?
    *   Hypothesis 2: API mismatch?
    *   Hypothesis 3: Missing dependency?
3.  **🔮 PREDICT**: If I change X, will it fix the test?
4.  **🛠️ ACTION**:
    *   **Verify**: `Read` the broken code first.
    *   **Edit**: Use `edit_file` to fix it.
    *   **Anti-Laziness**: You MUST write the **FULL** code. No `// ...` placeholders.

**Output Format**:
<thinking>
[Your analysis]
</thinking>
[Tool Calls]
</instructions>
",
            context = context,
            failed_tests = result.failed_tests.join("\n"),
            full_output = result.output.chars().take(4000).collect::<String>() // Increased limit
        )
    }
}

#[async_trait]
impl crate::agent::skills::SubAgent for AutoFixAgent {
    fn id(&self) -> &str {
        "auto_fix_agent"
    }

    fn name(&self) -> &str {
        "AutoFix Agent"
    }

    fn capabilities(&self) -> Vec<String> {
        vec!["auto_fix".to_string(), "test".to_string()]
    }

    fn can_handle(&self, task: &SubTask) -> bool {
        task.task_type == "auto_fix" || task.task_type == "test"
    }

    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        let config = AutoFixConfig {
            max_attempts: task.max_steps,
            test_command: task
                .params
                .get("test_command")
                .and_then(|v| v.as_str())
                .unwrap_or("cargo test")
                .to_string(),
            timeout_secs: 600,
        };

        let result = self.run_loop(config, &task.objective).await?;
        let result_json = serde_json::to_value(&result).unwrap_or_default();

        Ok(SubTaskResult {
            task_id: task.id,
            success: result.success,
            summary: if result.success {
                "Tests passed".to_string()
            } else {
                "Tests failed".to_string()
            },
            details: Some(result.output),
            data: Some(result_json),
            next_action: None,
            error: result.error,
        })
    }
}
