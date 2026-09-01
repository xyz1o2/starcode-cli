use super::{SubAgent, SubTask, SubTaskResult};
use crate::core::prompts::skills::search::SEARCH_SYSTEM_PROMPT;
use crate::agent::StarAgent;
use crate::core::config::Config;
use crate::core::tools::semantic_search::run_semantic_search_for_skill;
use crate::core::utils::paths::resolve_tool_path;
use crate::llm::client::StarClient;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub struct SearchAgent {
    id: String,
    client: StarClient,
    config: Arc<Config>,
}

impl SearchAgent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        Self {
            id: "Grep".to_string(),
            client,
            config,
        }
    }

    async fn run_search_loop(
        &self,
        task: &SubTask,
    ) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        // Step 1: Broad Search (ACE Inverted Index)
        // Use objective as query; path should not pollute semantic terms.
        let query = task.objective.trim().to_string();
        crate::utils::logging::append_debug_log_line(&format!(
            "🔍 SearchAgent: Executing Broad Search for '{}'",
            query
        ));

        let target = task.target.trim();
        let requested_root = if target.is_empty() || target == "." {
            self.config.target_dir().clone()
        } else {
            resolve_tool_path(self.config.target_dir(), target)
        };
        let (root_path, root_note) = if requested_root.exists() && requested_root.is_dir() {
            (requested_root, None)
        } else {
            (
                self.config.target_dir().clone(),
                Some(format!(
                    "Requested search path '{}' is invalid; fallback to workspace root '{}'.",
                    requested_root.display(),
                    self.config.target_dir().display()
                )),
            )
        };
        let search_root_display = root_path.display().to_string();

        let mut broad_search_results =
            run_semantic_search_for_skill(root_path.clone(), query.clone()).await;
        if let Some(note) = root_note {
            broad_search_results = format!("{}\n\n{}", note, broad_search_results);
        }

        let deep_filter_enabled = search_skill_deep_filter_enabled(task);
        if !deep_filter_enabled {
            return Ok(SubTaskResult::success(
                task.id.clone(),
                "Search Complete (fast path)".to_string(),
            )
            .with_details(format!(
                "Search Root: {}\nMode: deterministic fast path\n\n{}",
                search_root_display, broad_search_results
            ))
            .with_data(json!({
                "mode": "fast_path",
                "search_root": search_root_display,
                "query": query,
                "deep_filter": false,
            })));
        }

        // Step 2: Deep Filter (LLM Agent)
        // We initialize a sub-agent to process the results and potentially dig deeper
        // Prefer using the current model to maintain consistency with user's choice
        let model = self.client.model.clone();

        let mut agent = match StarAgent::new(
            &self.client.api_key,
            Some(model),
            self.client.base_url.clone(),
            Some(10),
            Some(self.client.is_openai_compatible),
            Some(self.config.clone()),
        )
        .await
        {
            Ok(agent) => agent,
            Err(e) => {
                let mut details = broad_search_results.clone();
                details.push_str(&format!("\n\n[Search agent init error: {}]", e));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Search Complete (fallback)".to_string(),
                )
                .with_details(details));
            }
        };

        // Construct a prompt that includes the Broad Search results as "Pre-fetched Context"
        let prompt = format!(
            "{}\n\n\
            ## CURRENT TASK\n\
            User Query: {}\n\
            Search Root: {}\n\
            Params: {:?}\n\n\
            ## BROAD SEARCH RESULTS (ACE Engine)\n\
            The following results were retrieved from the Inverted Index (Fast Context). \
            Review them carefully. \
            1. If you see the answer directly in the snippets, extract it and answer the user.\n\
            2. If the snippets are truncated or you need more context, use `Read` with `offset` and `limit` to investigate specific files mentioned below.\n\
            3. Do NOT call `semantic_search` again. Use the file paths provided below as starting points.\n\
            4. Prefer `Read` first. Use `Grep` or `Glob` only if the listed files are insufficient.\n\
            5. PARALLEL EXECUTION: If you need to read multiple files, generate multiple `Read` calls in a single turn.\n\
            6. Keep the answer concise and grounded in concrete file paths.\n\
            --------------------------------------------------\n\
            {}\n\
            --------------------------------------------------\n\
            ",
            SEARCH_SYSTEM_PROMPT,
            task.objective,
            search_root_display,
            task.params,
            broad_search_results
        );

        crate::utils::logging::append_debug_log_line(
            "🔍 SearchAgent: Starting Deep Filter Analysis...",
        );

        let entries = match tokio::time::timeout(
            search_skill_deep_filter_timeout(),
            agent.process_user_message(&prompt),
        )
        .await
        {
            Ok(Ok(entries)) => entries,
            Ok(Err(e)) => {
                let mut details = broad_search_results.clone();
                details.push_str(&format!("\n\n[Search agent error: {}]", e));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Search Complete (fallback)".to_string(),
                )
                .with_details(details)
                .with_data(json!({
                    "mode": "deep_filter_fallback",
                    "search_root": search_root_display,
                    "query": query,
                    "deep_filter": true,
                    "fallback_reason": format!("{}", e),
                })));
            }
            Err(_) => {
                let mut details = broad_search_results.clone();
                details.push_str(&format!(
                    "\n\n[Search agent timed out after {}ms; returning broad search results.]",
                    search_skill_deep_filter_timeout().as_millis()
                ));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Search Complete (timeout fallback)".to_string(),
                )
                .with_details(details)
                .with_data(json!({
                    "mode": "deep_filter_timeout_fallback",
                    "search_root": search_root_display,
                    "query": query,
                    "deep_filter": true,
                    "timeout_ms": search_skill_deep_filter_timeout().as_millis(),
                })));
            }
        };

        let response = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == crate::types::ChatEntryType::Assistant)
            .map(|e| e.content.clone())
            .unwrap_or_else(|| "No response".to_string());

        let final_details = if response.trim().is_empty() || response == "No response" {
            broad_search_results
        } else {
            response
        };

        Ok(
            SubTaskResult::success(task.id.clone(), "Search Complete".to_string())
                .with_details(final_details)
                .with_data(json!({
                    "mode": "deep_filter",
                    "search_root": search_root_display,
                    "query": query,
                    "deep_filter": true,
                })),
        )
    }
}

fn parse_bool_like(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(v) => Some(*v),
        serde_json::Value::String(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn task_bool_param(task: &SubTask, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| task.params.get(*key).and_then(parse_bool_like))
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn search_skill_deep_filter_enabled(task: &SubTask) -> bool {
    search_skill_deep_filter_enabled_with_default(
        task,
        env_bool("STAR_SEARCH_SKILL_ENABLE_DEEP_FILTER", false),
    )
}

fn search_skill_deep_filter_enabled_with_default(task: &SubTask, env_default: bool) -> bool {
    task_bool_param(task, &["deep_filter", "deep_analysis", "agentic"]).unwrap_or(env_default)
}

fn search_skill_deep_filter_timeout() -> Duration {
    let timeout_ms = std::env::var("STAR_SEARCH_SKILL_DEEP_FILTER_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15_000);
    Duration::from_millis(timeout_ms.max(1_000))
}

#[async_trait]
impl SubAgent for SearchAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Search Agent (检索专家)"
    }

    fn capabilities(&self) -> Vec<String> {
        vec![
            "Grep".to_string(),
            "find".to_string(),
            "query".to_string(),
            "locate".to_string(),
        ]
    }

    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        self.run_search_loop(&task).await
    }
}
 