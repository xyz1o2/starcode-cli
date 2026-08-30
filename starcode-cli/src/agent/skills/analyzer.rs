/// Analyzer Agent - 代码分析专家
///
/// 职责：
/// - 分析代码结构
/// - 查找依赖关系  
/// - 检测代码问题
/// - 提取符号定义
///
/// 使用的工具：
/// - search (ripgrep)
/// - Read / read_many_files
/// - grep
use super::{SubAgent, SubTask, SubTaskResult};
use crate::core::prompts::skills::analyzer::ANALYZER_SYSTEM_PROMPT;
use crate::agent::StarAgent;
use crate::core::config::Config;
use crate::core::tools::project_map::run_project_map_for_skill;
use crate::core::utils::paths::resolve_tool_path;
use crate::llm::client::StarClient;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub struct AnalyzerAgent {
    id: String,
    client: StarClient,
    config: Arc<Config>,
}

impl AnalyzerAgent {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        Self {
            id: "analyzer".to_string(),
            client,
            config,
        }
    }

    /// 执行 LLM 驱动的分析
    async fn run_analysis_loop(
        &self,
        task: &SubTask,
    ) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
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
                    "Requested analysis path '{}' is invalid; fallback to workspace root '{}'.",
                    requested_root.display(),
                    self.config.target_dir().display()
                )),
            )
        };
        let search_root_display = root_path.display().to_string();
        let max_depth = parse_usize_param(&task.params, "max_depth", 4).clamp(2, 8);
        let max_files = parse_usize_param(&task.params, "max_files", 260).clamp(80, 1200);
        let include_symbols = task_bool_param(&task.params, &["include_symbols"]).unwrap_or(false);

        let mut project_map_output =
            run_project_map_for_skill(root_path.clone(), max_depth, max_files, include_symbols)
                .await;
        if let Some(note) = root_note {
            project_map_output = format!("{}\n\n{}", note, project_map_output);
        }

        let fast_path_details = format!(
            "Analysis Root: {}\nMode: deterministic fast path\n\n{}\n\nSuggested next steps:\n- Ask for one concrete module/file/function if you want a precise deep dive.\n- Enable deep_filter only when you need synthesized risks or architecture commentary.",
            search_root_display, project_map_output
        );

        if !analyzer_deep_filter_enabled(task) {
            return Ok(SubTaskResult::success(
                task.id.clone(),
                "Analysis Complete (fast path)".to_string(),
            )
            .with_details(fast_path_details)
            .with_data(json!({
                "mode": "fast_path",
                "search_root": search_root_display,
                "max_depth": max_depth,
                "max_files": max_files,
                "include_symbols": include_symbols,
                "deep_filter": false,
            })));
        }

        // 创建一个专注于分析的 Agent
        let mut agent = StarAgent::new(
            &self.client.api_key,
            Some(self.client.model.clone()),
            self.client.base_url.clone(),
            Some(10), // Limit turns for sub-task
            Some(self.client.is_openai_compatible),
            Some(self.config.clone()),
        )
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        // 注入专用 System Prompt
        // 注意：StarAgent 目前没有直接设置 System Prompt 的公开 API，
        // 但我们可以通过发送第一条 User 消息带上 System 指令，或者修改 StarAgent 以支持 Override。
        // 为了简单起见，我们将 System Prompt 作为 User 消息的前缀，或者依靠 Agent 内部的 Router。
        // 更优雅的方式是：StarAgent::new 应该允许传入 System Prompt Override。
        // 但根据现有 API，我们构建一个明确的任务描述。

        let prompt = format!(
            "{}\n\nTask: Analyze the following target.\nObjective: {}\nTarget: {}\nParams: {:?}\n\n## DETERMINISTIC PROJECT MAP\n{}\n\nConstraints:\n1. Do not call `ProjectMap` again.\n2. Prefer `Read` for verification before making claims.\n3. Keep the answer grounded in file paths and concrete architecture notes.\n4. Focus on risks, architecture, and quick wins only when supported by the map.",
            ANALYZER_SYSTEM_PROMPT, task.objective, task.target, task.params, project_map_output
        );

        let entries = match tokio::time::timeout(
            analyzer_deep_filter_timeout(),
            agent.process_user_message(&prompt),
        )
        .await
        {
            Ok(Ok(entries)) => entries,
            Ok(Err(err)) => {
                let mut details = fast_path_details.clone();
                details.push_str(&format!("\n\n[Analyzer error: {}]", err));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Analysis Complete (fallback)".to_string(),
                )
                .with_details(details)
                .with_data(json!({
                    "mode": "deep_filter_fallback",
                    "search_root": search_root_display,
                    "max_depth": max_depth,
                    "max_files": max_files,
                    "include_symbols": include_symbols,
                    "deep_filter": true,
                    "fallback_reason": format!("{}", err),
                })));
            }
            Err(_) => {
                let mut details = fast_path_details.clone();
                details.push_str(&format!(
                    "\n\n[Analyzer timed out after {}ms; returning project map summary.]",
                    analyzer_deep_filter_timeout().as_millis()
                ));
                return Ok(SubTaskResult::success(
                    task.id.clone(),
                    "Analysis Complete (timeout fallback)".to_string(),
                )
                .with_details(details)
                .with_data(json!({
                    "mode": "deep_filter_timeout_fallback",
                    "search_root": search_root_display,
                    "max_depth": max_depth,
                    "max_files": max_files,
                    "include_symbols": include_symbols,
                    "deep_filter": true,
                    "timeout_ms": analyzer_deep_filter_timeout().as_millis(),
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
            fast_path_details
        } else {
            response
        };

        Ok(
            SubTaskResult::success(task.id.clone(), "Analysis Complete".to_string())
                .with_details(final_details)
                .with_data(json!({
                    "mode": "deep_filter",
                    "search_root": search_root_display,
                    "max_depth": max_depth,
                    "max_files": max_files,
                    "include_symbols": include_symbols,
                    "deep_filter": true,
                })),
        )
    }
}

fn parse_usize_param(
    params: &std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    default: usize,
) -> usize {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(default)
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

fn task_bool_param(
    params: &std::collections::HashMap<String, serde_json::Value>,
    keys: &[&str],
) -> Option<bool> {
    keys.iter()
        .find_map(|key| params.get(*key).and_then(parse_bool_like))
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

fn analyzer_deep_filter_enabled(task: &SubTask) -> bool {
    task_bool_param(&task.params, &["deep_filter", "deep_analysis", "agentic"])
        .unwrap_or_else(|| env_bool("STAR_ANALYZER_ENABLE_DEEP_FILTER", false))
}

fn analyzer_deep_filter_timeout() -> Duration {
    let timeout_ms = std::env::var("STAR_ANALYZER_DEEP_FILTER_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(18_000);
    Duration::from_millis(timeout_ms.max(1_000))
}

#[async_trait]
impl SubAgent for AnalyzerAgent {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Analyzer Agent (代码分析专家)"
    }

    fn capabilities(&self) -> Vec<String> {
        vec![
            "analyze".to_string(),
            "analysis".to_string(),
            "structure".to_string(),
            "dependency".to_string(),
            "review".to_string(),
        ]
    }

    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        self.run_analysis_loop(&task).await
    }
}
 