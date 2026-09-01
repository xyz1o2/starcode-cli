use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::tasks::manager::TaskManager;
use crate::core::tasks::models::TaskNode;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolInvocation, ToolLocation,
    ToolResult,
};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExitPlanModeParams {
    pub plan: String,
}

pub struct ExitPlanModeTool {
    config: Arc<Config>,
    message_bus: Arc<MessageBus>,
}

impl ExitPlanModeTool {
    pub fn new(config: Arc<Config>, message_bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            message_bus,
        }
    }
}

impl BaseDeclarativeTool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn display_name(&self) -> &str {
        "Exit Plan Mode"
    }

    fn description(&self) -> &str {
        "Prompts the user to exit plan mode and start coding. Use this when you have finished planning."
    }

    fn kind(&self) -> Kind {
        Kind::Other
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The plan you came up with, that you want to run by the user for approval. Supports markdown. The plan should be pretty concise."
                }
            },
            "required": ["plan"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ExitPlanModeParams = serde_json::from_value(params)?;
        Ok(Box::new(ExitPlanModeInvocation {
            params,
            config: self.config.clone(),
            message_bus: self.message_bus.clone(),
        }))
    }
}

pub struct ExitPlanModeInvocation {
    params: ExitPlanModeParams,
    config: Arc<Config>,
    message_bus: Arc<MessageBus>,
}

impl ToolInvocation for ExitPlanModeInvocation {
    fn get_description(&self) -> String {
        "Exit Plan Mode with plan".to_string()
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send,
        >,
    > {
        let plan = self.params.plan.clone();
        let message_bus = self.message_bus.clone();
        Box::pin(async move {
            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                title: "Exit Plan Mode?".to_string(),
                prompt: format!(
                    "The agent wants to exit Plan Mode and execute the following plan:\n\n{}",
                    plan
                ),
                on_confirm: std::sync::Arc::new(move |outcome| {
                    let confirmed = matches!(
                        outcome,
                        crate::types::ToolConfirmationOutcome::ProceedOnce
                            | crate::types::ToolConfirmationOutcome::ProceedAlways
                            | crate::types::ToolConfirmationOutcome::ProceedAlwaysAndSave
                            | crate::types::ToolConfirmationOutcome::AllowSession
                    );

                    if confirmed {
                        let bus = message_bus.clone();
                        tokio::spawn(async move {
                            bus.set_approval_mode(
                                crate::core::policy::types::ApprovalMode::Default,
                            )
                            .await;
                        });
                    }
                }),
            }))
        })
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let plan = self.params.plan.clone();
        let config = self.config.clone();
        Box::pin(async move {
            let seeded = seed_tasks_from_plan(&plan, &config).await?;
            let mut output = format!("Plan approved:\n{}", plan);
            match seeded {
                PlanSeedOutcome::Seeded { count, appended } => {
                    if appended {
                        output.push_str(&format!(
                            "\n\n任务列表已有内容，已追加为新计划节点（{} 个任务，Ctrl+B 查看任务面板）",
                            count
                        ));
                    } else {
                        output.push_str(&format!(
                            "\n\n已从计划生成 {} 个任务（Ctrl+B 查看任务面板）",
                            count
                        ));
                    }
                }
                PlanSeedOutcome::SkippedExisting => {
                    output.push_str("\n\n检测到相同计划已生成任务，未重复添加。");
                }
                PlanSeedOutcome::SkippedNoTasks => {
                    output.push_str("\n\n未识别计划中的任务清单，未生成任务。");
                }
            }
            Ok(ToolResult {
                llm_content: Some("User has approved the plan. There is nothing else needed from you now. Please respond with \"ok\"".to_string()),
                return_display: Some("Plan approved. Exiting Plan Mode.".to_string()),
                output,
                error: None,
                data: None,
            })
        })
    }
}

#[derive(Debug)]
enum PlanSeedOutcome {
    Seeded { count: usize, appended: bool },
    SkippedExisting,
    SkippedNoTasks,
}

#[derive(Debug, Serialize, Deserialize)]
struct PlanSeedMeta {
    plan_hash: u64,
    root_id: Option<String>,
}

#[derive(Debug, Clone)]
struct HeadingInfo {
    level: usize,
    title: String,
    has_items: bool,
}

#[derive(Debug, Clone)]
enum PlanLine {
    Heading {
        index: usize,
    },
    Item {
        indent: usize,
        title: String,
        heading_index: Option<usize>,
    },
}

#[derive(Debug, Clone)]
struct ParsedPlan {
    headings: Vec<HeadingInfo>,
    lines: Vec<PlanLine>,
    first_item_title: Option<String>,
    item_count: usize,
}

async fn seed_tasks_from_plan(
    plan: &str,
    config: &Config,
) -> Result<PlanSeedOutcome, Box<dyn std::error::Error>> {
    let plan = plan.trim();
    if plan.is_empty() {
        return Ok(PlanSeedOutcome::SkippedNoTasks);
    }

    let parsed = parse_plan(plan);
    if parsed.item_count == 0 {
        return Ok(PlanSeedOutcome::SkippedNoTasks);
    }

    let plan_hash = hash_plan(plan);
    let plan_title = derive_plan_title(&parsed);
    let cwd = config.working_dir().clone();
    let parsed_for_seed = parsed.clone();

    let outcome =
        tokio::task::spawn_blocking(move || -> Result<PlanSeedOutcome, std::io::Error> {
            let tasks_path = TaskManager::task_file_for_workspace(&cwd);
            let star_dir = tasks_path
                .parent()
                .map(|path| path.to_path_buf())
                .unwrap_or_else(|| cwd.join(".star"));
            fs::create_dir_all(&star_dir)?;
            let meta_path = plan_seed_meta_path(&cwd);

            let mut manager =
                TaskManager::load_from_file(&tasks_path).unwrap_or_else(|_| TaskManager::new());
            let has_tasks = !manager.graph.root_ids.is_empty();

            if has_tasks {
                if let Some(meta) = load_plan_seed_meta(&meta_path) {
                    if meta.plan_hash == plan_hash {
                        return Ok(PlanSeedOutcome::SkippedExisting);
                    }
                }
            }

            let mut heading_ids: Vec<Option<String>> = vec![None; parsed_for_seed.headings.len()];
            let mut heading_stack: Vec<(usize, usize)> = Vec::new();
            let mut list_stack: Vec<(usize, String)> = Vec::new();
            let mut current_heading_idx: Option<usize> = None;
            let mut seen: HashSet<String> = HashSet::new();
            let mut root_override: Option<String> = None;

            if has_tasks {
                let title = format!("Plan: {}", truncate_title(&plan_title, 60));
                let root = TaskNode::new(title);
                let root_id = root.id.clone();
                manager
                    .add_task(root)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                root_override = Some(root_id);
            }

            let mut created = 0usize;

            for line in &parsed_for_seed.lines {
                match line {
                    PlanLine::Heading { index } => {
                        let heading = &parsed_for_seed.headings[*index];
                        if !heading.has_items {
                            continue;
                        }
                        while let Some((level, _)) = heading_stack.last() {
                            if *level >= heading.level {
                                heading_stack.pop();
                            } else {
                                break;
                            }
                        }
                        let parent_id = heading_stack
                            .last()
                            .and_then(|(_, idx)| heading_ids[*idx].clone())
                            .or_else(|| root_override.clone());
                        let mut node = TaskNode::new(heading.title.clone());
                        if let Some(pid) = parent_id {
                            node.parent_id = Some(pid);
                        }
                        let id = node.id.clone();
                        manager
                            .add_task(node)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                        heading_ids[*index] = Some(id);
                        heading_stack.push((heading.level, *index));
                        current_heading_idx = Some(*index);
                        list_stack.clear();
                    }
                    PlanLine::Item {
                        indent,
                        title,
                        heading_index,
                    } => {
                        if *heading_index != current_heading_idx {
                            current_heading_idx = *heading_index;
                            list_stack.clear();
                        }

                        let norm = normalize_title(title);
                        if norm.is_empty() {
                            continue;
                        }
                        let key = format!(
                            "{}::{}",
                            heading_index
                                .map(|i| i.to_string())
                                .unwrap_or_else(|| "root".to_string()),
                            norm
                        );
                        if !seen.insert(key) {
                            continue;
                        }

                        let base_parent = heading_index
                            .and_then(|i| heading_ids[i].clone())
                            .or_else(|| root_override.clone());

                        while let Some((prev_indent, _)) = list_stack.last() {
                            if *indent <= *prev_indent {
                                list_stack.pop();
                            } else {
                                break;
                            }
                        }

                        let parent_id = list_stack.last().map(|(_, id)| id.clone()).or(base_parent);
                        let mut node = TaskNode::new(title.clone());
                        if let Some(pid) = parent_id {
                            node.parent_id = Some(pid);
                        }
                        let id = node.id.clone();
                        manager
                            .add_task(node)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                        list_stack.push((*indent, id));
                        created += 1;
                    }
                }
            }

            if created == 0 {
                return Ok(PlanSeedOutcome::SkippedNoTasks);
            }

            manager
                .save_to_file(&tasks_path)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            let meta = PlanSeedMeta {
                plan_hash,
                root_id: root_override,
            };
            save_plan_seed_meta(&meta_path, &meta)?;

            Ok(PlanSeedOutcome::Seeded {
                count: created,
                appended: has_tasks,
            })
        })
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)??;

    Ok(outcome)
}

fn parse_plan(plan: &str) -> ParsedPlan {
    let mut headings: Vec<HeadingInfo> = Vec::new();
    let mut lines: Vec<PlanLine> = Vec::new();
    let mut current_heading: Option<usize> = None;
    let mut in_code_block = false;
    let mut first_item_title: Option<String> = None;
    let mut item_count = 0usize;

    for raw in plan.lines() {
        let line = raw.trim_end();
        let content = line.trim_start();
        if content.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || content.is_empty() {
            continue;
        }

        if let Some((level, title)) = parse_heading(content) {
            let index = headings.len();
            headings.push(HeadingInfo {
                level,
                title,
                has_items: false,
            });
            lines.push(PlanLine::Heading { index });
            current_heading = Some(index);
            continue;
        }

        if let Some((indent, title)) = parse_list_item(line) {
            if first_item_title.is_none() {
                first_item_title = Some(title.clone());
            }
            if let Some(idx) = current_heading {
                if let Some(heading) = headings.get_mut(idx) {
                    heading.has_items = true;
                }
            }
            lines.push(PlanLine::Item {
                indent,
                title,
                heading_index: current_heading,
            });
            item_count += 1;
        }
    }

    ParsedPlan {
        headings,
        lines,
        first_item_title,
        item_count,
    }
}

fn parse_heading(line: &str) -> Option<(usize, String)> {
    let bytes = line.as_bytes();
    let mut level = 0usize;
    while level < bytes.len() && bytes[level] == b'#' {
        level += 1;
    }
    if level == 0 || level > 6 {
        return None;
    }
    if level >= bytes.len() || bytes[level] != b' ' {
        return None;
    }
    let title = line[level + 1..].trim();
    if title.is_empty() {
        return None;
    }
    Some((level, title.to_string()))
}

fn parse_list_item(line: &str) -> Option<(usize, String)> {
    let (indent, trimmed) = split_indent(line);
    if trimmed.is_empty() {
        return None;
    }

    for prefix in ["- ", "* ", "+ ", "• "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = strip_checkbox(rest).trim();
            if !rest.is_empty() {
                return Some((indent, rest.to_string()));
            }
            return None;
        }
    }

    if let Some(rest) = strip_numeric_prefix(trimmed) {
        let rest = strip_checkbox(rest.as_str()).trim();
        if !rest.is_empty() {
            return Some((indent, rest.to_string()));
        }
        return None;
    }

    if let Some(rest) = strip_chinese_list_prefix(trimmed) {
        let rest = strip_checkbox(rest.as_str()).trim();
        if !rest.is_empty() {
            return Some((indent, rest.to_string()));
        }
        return None;
    }

    None
}

fn split_indent(line: &str) -> (usize, &str) {
    let mut indent = 0usize;
    let mut idx = 0usize;
    for (i, ch) in line.char_indices() {
        match ch {
            ' ' => {
                indent += 1;
                idx = i + ch.len_utf8();
            }
            '\t' => {
                indent += 4;
                idx = i + ch.len_utf8();
            }
            _ => break,
        }
    }
    (indent, &line[idx..])
}
fn strip_numeric_prefix(input: &str) -> Option<String> {
    let mut chars = input.chars().peekable();
    let mut digit_seen = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            digit_seen = true;
            chars.next();
        } else {
            break;
        }
    }
    if !digit_seen {
        return None;
    }

    let sep = match chars.peek() {
        Some('.') | Some(')') | Some('、') | Some(':') | Some('：') => chars.next(),
        _ => None,
    };
    if sep.is_none() {
        return None;
    }

    if let Some(&next) = chars.peek() {
        if next.is_whitespace() {
            chars.next();
        }
    }
    let rest: String = chars.collect();
    let rest = rest.trim();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

fn strip_chinese_list_prefix(input: &str) -> Option<String> {
    const NUMS: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    const SEPS: [&str; 6] = ["、", ".", "．", ")", "）", "："];

    for num in NUMS {
        for sep in SEPS {
            let prefix = format!("{}{}", num, sep);
            if let Some(rest) = input.strip_prefix(&prefix) {
                let rest = rest.trim_start();
                if rest.is_empty() {
                    return None;
                }
                return Some(rest.to_string());
            }
        }
    }
    None
}
fn strip_checkbox(input: &str) -> &str {
    if let Some(rest) = input.strip_prefix("[ ] ") {
        return rest;
    }
    if let Some(rest) = input.strip_prefix("[x] ") {
        return rest;
    }
    if let Some(rest) = input.strip_prefix("[X] ") {
        return rest;
    }
    input
}

fn normalize_title(title: &str) -> String {
    let trimmed = title
        .trim()
        .trim_end_matches(|c: char| matches!(c, ':' | '：' | ';' | '；' | '.' | '。'));
    let mut out = String::new();
    let mut last_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            if ch.is_ascii_alphabetic() {
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
            last_space = false;
        }
    }
    out.trim().to_string()
}

fn truncate_title(title: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (i, ch) in title.chars().enumerate() {
        if i >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

fn derive_plan_title(parsed: &ParsedPlan) -> String {
    if let Some(h) = parsed.headings.first() {
        return h.title.clone();
    }
    if let Some(item) = &parsed.first_item_title {
        return item.clone();
    }
    "Plan".to_string()
}

fn hash_plan(plan: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    plan.hash(&mut hasher);
    hasher.finish()
}

fn plan_seed_meta_path(cwd: &Path) -> PathBuf {
    cwd.join(".star").join("plan_task_seed.json")
}

fn load_plan_seed_meta(path: &Path) -> Option<PlanSeedMeta> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_plan_seed_meta(path: &Path, meta: &PlanSeedMeta) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(meta)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(path, content)
}
