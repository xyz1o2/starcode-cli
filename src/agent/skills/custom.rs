use super::loader::{SkillLoader, SkillMetadata};
use super::{SubAgent, SubAgentManager, SubTask, SubTaskResult};
use crate::agent::StarAgent;
use crate::core::config::storage::Storage;
use crate::core::config::Config;
use crate::llm::client::StarClient;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::loader::execute_shell_in_prompt;

#[derive(Debug, Clone)]
pub struct CustomSubAgentDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    pub aliases: Vec<String>,
    pub model: Option<String>,
    pub prompt: String,
    pub source_path: PathBuf,
    pub capabilities: Vec<String>,
    pub metadata: SkillMetadata,
}

pub struct CustomSubAgent {
    definition: CustomSubAgentDefinition,
    client: StarClient,
    config: Arc<Config>,
    alias_set: HashSet<String>,
}

impl CustomSubAgent {
    pub fn new(
        definition: CustomSubAgentDefinition,
        client: StarClient,
        config: Arc<Config>,
    ) -> Self {
        let mut alias_set = HashSet::new();
        alias_set.insert(definition.id.clone());
        for alias in &definition.aliases {
            alias_set.insert(normalize_custom_agent_id(alias));
        }
        Self {
            definition,
            client,
            config,
            alias_set,
        }
    }

    fn score_task(&self, task: &SubTask) -> i32 {
        let task_type = normalize_custom_agent_id(&task.task_type);
        let objective = task.objective.to_lowercase();
        let target = task.target.to_lowercase();
        let mut score = 0i32;

        if task_type == self.definition.id {
            score += 120;
        } else if self.alias_set.contains(&task_type) {
            score += 100;
        }

        if objective.contains(&self.definition.id) {
            score += 80;
        }

        for alias in &self.alias_set {
            if !alias.is_empty() && objective.contains(alias) {
                score += 70;
            }
            if !alias.is_empty() && target.contains(alias) {
                score += 30;
            }
        }

        for token in &self.definition.capabilities {
            if token.len() >= 4 && objective.contains(token) {
                score += 12;
            }
        }

        score
    }
}

#[async_trait]
impl SubAgent for CustomSubAgent {
    fn id(&self) -> &str {
        &self.definition.id
    }

    fn name(&self) -> &str {
        &self.definition.name
    }

    fn capabilities(&self) -> Vec<String> {
        self.definition.capabilities.clone()
    }

    fn can_handle(&self, task: &SubTask) -> bool {
        // Check paths constraint: if the skill has paths, only activate for matching files
        if !self.definition.metadata.paths.is_empty() {
            let file_paths: Vec<String> = vec![task.target.clone()];
            if !SkillLoader::should_activate_skill(&self.definition.metadata.paths, &file_paths) {
                return false;
            }
        }
        self.score_task(task) > 0
    }

    fn match_score(&self, task: &SubTask) -> i32 {
        if !self.definition.metadata.paths.is_empty() {
            let file_paths: Vec<String> = vec![task.target.clone()];
            if !SkillLoader::should_activate_skill(&self.definition.metadata.paths, &file_paths) {
                return 0;
            }
        }
        self.score_task(task)
    }

    async fn execute(&self, task: SubTask) -> Result<SubTaskResult, Box<dyn std::error::Error>> {
        let max_turns = (task.max_steps.max(2) as u32).saturating_add(2);
        let mut agent = StarAgent::new(
            &self.client.api_key,
            self.definition
                .model
                .clone()
                .or_else(|| Some(self.client.model.clone())),
            self.client.base_url.clone(),
            Some(max_turns),
            Some(self.client.is_openai_compatible),
            Some(self.config.clone()),
        )
        .await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        // Build the prompt with parameter substitution
        let mut prompt_body = self.definition.prompt.clone();

        // Collect positional args from task params if available
        let positional_args: Vec<String> = task
            .params
            .get("positional_args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // Collect named args from task params
        let named_args: HashMap<String, String> = task
            .params
            .get("named_args")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        // Perform parameter substitution
        prompt_body =
            SkillLoader::substitute_parameters(&prompt_body, &positional_args, &named_args);

        // Execute inline shell commands
        prompt_body = execute_shell_in_prompt(&prompt_body);

        let prompt = format!(
            "{}\n\n[SubTask]\nObjective: {}\nTarget: {}\nParams: {:?}",
            prompt_body, task.objective, task.target, task.params
        );

        let entries = agent
            .process_user_message(&prompt)
            .await
            .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

        let response = entries
            .iter()
            .rev()
            .find(|e| e.entry_type == crate::types::ChatEntryType::Assistant)
            .map(|e| e.content.clone())
            .unwrap_or_else(|| "No response".to_string());

        Ok(SubTaskResult::success(
            task.id.clone(),
            format!("Custom agent '{}' completed", self.definition.id),
        )
        .with_details(response))
    }
}

pub fn register_custom_subagents(
    manager: &mut SubAgentManager,
    client: StarClient,
    config: Arc<Config>,
) -> Vec<CustomSubAgentDefinition> {
    let defs = load_custom_subagent_definitions(config.target_dir());
    for def in defs.iter().cloned() {
        manager.register(Box::new(CustomSubAgent::new(
            def,
            client.clone(),
            config.clone(),
        )));
    }
    defs
}

pub fn load_custom_subagent_definitions(project_root: &Path) -> Vec<CustomSubAgentDefinition> {
    let storage = Storage::new(project_root.to_path_buf());
    let user_dir = Storage::user_agents_dir();
    let project_dir = storage.project_agents_dir();

    // Load user first, then let project override by id.
    let mut by_id: HashMap<String, CustomSubAgentDefinition> = HashMap::new();
    for dir in [user_dir, project_dir] {
        for path in list_agent_files(&dir) {
            if let Some(def) = load_custom_subagent_from_file(&path) {
                by_id.insert(def.id.clone(), def);
            }
        }
    }

    let mut defs: Vec<CustomSubAgentDefinition> = by_id.into_values().collect();
    defs.sort_by(|a, b| a.id.cmp(&b.id));
    defs
}

pub fn load_custom_subagent_from_file(path: &Path) -> Option<CustomSubAgentDefinition> {
    let content = std::fs::read_to_string(path).ok()?;
    let (meta, body) = SkillLoader::parse_frontmatter(&content);
    if meta.disabled {
        return None;
    }

    let file_stem = path.file_stem()?.to_string_lossy().to_string();

    // For custom subagents, check for id in the legacy simple-key format first,
    // but also support the new metadata format
    let id_from_meta = if meta.name.is_empty() {
        None
    } else {
        // Use name as id fallback if no explicit id was parsed
        // (the new frontmatter doesn't have an 'id' field, so we use file_stem)
        None
    };

    // Re-parse with legacy format to get 'id' and 'aliases' fields
    // that the new SkillMetadata doesn't include
    let (legacy_meta, _) = parse_legacy_frontmatter(&content);

    let id_raw = legacy_meta
        .get("id")
        .cloned()
        .or(id_from_meta)
        .unwrap_or(file_stem.clone());
    let id = normalize_custom_agent_id(&id_raw);
    if id.is_empty() {
        return None;
    }

    let name = if meta.name.is_empty() {
        file_stem.clone()
    } else {
        meta.name.clone()
    };
    let description = if meta.description.is_empty() {
        infer_description_from_prompt(&body, &name)
    } else {
        meta.description.clone()
    };

    let tools = if meta.allowed_tools.is_empty() {
        parse_csv_field(legacy_meta.get("tools").or_else(|| legacy_meta.get("tool")))
    } else {
        meta.allowed_tools.clone()
    };
    let aliases = parse_csv_field(legacy_meta.get("aliases").or_else(|| legacy_meta.get("alias")));
    let model = meta.model.clone();
    let prompt = body.trim().to_string();
    let prompt = if prompt.is_empty() {
        format!(
            "You are custom subagent '{}'. Focus on: {}.",
            name, description
        )
    } else {
        prompt
    };

    let capabilities = build_capabilities(&id, &name, &description, &tools, &aliases);

    Some(CustomSubAgentDefinition {
        id,
        name,
        description,
        tools,
        aliases,
        model,
        prompt,
        source_path: path.to_path_buf(),
        capabilities,
        metadata: meta,
    })
}

/// Legacy frontmatter parser for backward compatibility with simple key-value fields
fn parse_legacy_frontmatter(content: &str) -> (HashMap<String, String>, String) {
    let mut meta = HashMap::new();
    let mut lines = content.lines();
    let first = lines.next().unwrap_or_default().trim().to_string();
    if first != "---" {
        return (meta, content.to_string());
    }

    let mut fm_lines: Vec<String> = Vec::new();
    let mut body_lines: Vec<String> = Vec::new();
    let mut in_frontmatter = true;

    for line in lines {
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
            continue;
        }
        if in_frontmatter {
            fm_lines.push(line.to_string());
        } else {
            body_lines.push(line.to_string());
        }
    }

    for line in fm_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let key = k.trim().to_string();
            let value = v.trim().trim_matches('"').trim_matches('\'').to_string();
            // Only insert if it's a simple scalar value (not empty, meaning no array follows)
            if !value.is_empty() {
                meta.insert(key, value);
            }
        }
    }

    (meta, body_lines.join("\n"))
}

pub fn render_custom_subagent_markdown(
    id: &str,
    name: &str,
    description: &str,
    tools: &[String],
    aliases: &[String],
    model: Option<&str>,
    prompt: &str,
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("id: {}\n", id));
    out.push_str(&format!("name: {}\n", name));
    out.push_str(&format!("description: {}\n", description));
    if !tools.is_empty() {
        out.push_str(&format!("tools: {}\n", tools.join(", ")));
    }
    if !aliases.is_empty() {
        out.push_str(&format!("aliases: {}\n", aliases.join(", ")));
    }
    if let Some(m) = model {
        if !m.trim().is_empty() {
            out.push_str(&format!("model: {}\n", m.trim()));
        }
    }
    out.push_str("---\n\n");
    out.push_str(prompt.trim());
    out.push('\n');
    out
}

pub fn normalize_custom_agent_id(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '@' {
                Some(c)
            } else if c.is_whitespace() {
                Some('_')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn list_agent_files(dir: &Path) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.ends_with(".md") || name.ends_with(".markdown") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn parse_csv_field(raw: Option<&String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let normalized = raw.trim().trim_start_matches('[').trim_end_matches(']');
    let mut out = Vec::new();
    for part in normalized.split(',') {
        let item = part
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string();
        if !item.is_empty() {
            out.push(item);
        }
    }
    out
}

fn is_truthy(input: &str) -> bool {
    matches!(
        input.trim().to_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn infer_description_from_prompt(prompt: &str, fallback_name: &str) -> String {
    for line in prompt.lines() {
        let cleaned = line
            .trim()
            .trim_start_matches('#')
            .trim_start_matches('-')
            .trim();
        if !cleaned.is_empty() {
            return cleaned.chars().take(96).collect();
        }
    }
    format!("Custom subagent '{}'", fallback_name)
}

fn build_capabilities(
    id: &str,
    name: &str,
    description: &str,
    tools: &[String],
    aliases: &[String],
) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();

    for value in [id.to_string(), name.to_string(), description.to_string()] {
        for token in tokenize(&value) {
            set.insert(token);
        }
    }
    for alias in aliases {
        for token in tokenize(alias) {
            set.insert(token);
        }
    }
    for tool in tools {
        for token in tokenize(tool) {
            set.insert(token);
        }
    }

    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .filter_map(|token| {
            let t = token.trim_matches('_').trim_matches('-').trim();
            if t.len() >= 2 {
                Some(t.to_string())
            } else {
                None
            }
        })
        .collect()
}
 