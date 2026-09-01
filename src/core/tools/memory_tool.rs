use crate::core::confirmation_bus::MessageBus;
use crate::core::tools::diff_options::create_patch;
use crate::core::tools::modifiable_tool::{ModifiableDeclarativeTool, ModifyContext};
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use crate::core::utils::paths::{current_project_star_dir, tildeify_path};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

pub const DEFAULT_CONTEXT_FILENAME: &str = "STARCODE.md";
pub const MEMORY_SECTION_HEADER: &str = "## StarCode Added Memories";

use std::sync::Mutex;

static CURRENT_STARCODE_MD_FILENAME: Mutex<Option<String>> = Mutex::new(None);

pub fn set_starcode_md_filename(new_filename: &str) {
    if !new_filename.trim().is_empty() {
        if let Ok(mut lock) = CURRENT_STARCODE_MD_FILENAME.lock() {
            *lock = Some(new_filename.trim().to_string());
        }
    }
}

pub fn get_current_starcode_md_filename() -> String {
    CURRENT_STARCODE_MD_FILENAME
        .lock()
        .ok()
        .and_then(|lock| lock.clone())
        .unwrap_or_else(|| DEFAULT_CONTEXT_FILENAME.to_string())
}

pub fn get_global_memory_file_path() -> PathBuf {
    current_project_star_dir().join(get_current_starcode_md_filename())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveMemoryParams {
    pub fact: String,
    pub category: Option<String>,
    #[serde(rename = "modified_by_user")]
    pub modified_by_user: Option<bool>,
    #[serde(rename = "modified_content")]
    pub modified_content: Option<String>,
}

fn ensure_newline_separation(current_content: &str) -> String {
    if current_content.is_empty() {
        String::new()
    } else if current_content.ends_with("\n\n") || current_content.ends_with("\r\n\r\n") {
        String::new()
    } else if current_content.ends_with('\n') || current_content.ends_with("\r\n") {
        "\n".to_string()
    } else {
        "\n\n".to_string()
    }
}

async fn read_memory_file_content() -> Result<String, Box<dyn std::error::Error>> {
    let path = get_global_memory_file_path();

    match tokio::fs::read_to_string(&path).await {
        Ok(content) => Ok(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

fn get_header_for_category(category: Option<&str>) -> String {
    match category.as_deref() {
        Some("user") => "## User Preferences".to_string(),
        Some("project") => "## Project Context".to_string(),
        Some(c) => format!("## {} Memories", c),
        None => MEMORY_SECTION_HEADER.to_string(),
    }
}

fn compute_new_content(current_content: &str, fact: &str, category: Option<&str>) -> String {
    let mut processed_text = fact.trim().to_string();
    processed_text = processed_text
        .trim_start_matches(|c| c == '-')
        .trim()
        .to_string();
    let new_memory_item = format!("- {}", processed_text);

    let header = get_header_for_category(category);
    let header_index = current_content.find(&header);

    if let Some(header_pos) = header_index {
        let start_of_section = header_pos + header.len();
        let end_of_section = current_content[start_of_section..]
            .find("\n## ")
            .map(|pos| start_of_section + pos)
            .unwrap_or(current_content.len());

        let before_section = current_content[..start_of_section].trim_end().to_string();
        let mut section_content = current_content[start_of_section..end_of_section]
            .trim_end()
            .to_string();
        let after_section = current_content[end_of_section..].to_string();

        section_content.push_str(&format!("\n{}", new_memory_item));

        format!(
            "{}\n{}\n{}\n",
            before_section,
            section_content.trim_start(),
            after_section
        )
    } else {
        let separator = ensure_newline_separation(current_content);
        format!(
            "{}{}{}\n{}\n",
            current_content, separator, header, new_memory_item
        )
    }
}

pub struct MemoryToolInvocation {
    params: SaveMemoryParams,
    allowlist: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl MemoryToolInvocation {
    pub fn new(
        params: SaveMemoryParams,
        _message_bus: Arc<MessageBus>,
        _tool_name: Option<String>,
        _tool_display_name: Option<String>,
        allowlist: Arc<std::sync::Mutex<HashSet<String>>>,
    ) -> Self {
        Self { params, allowlist }
    }
}

impl ToolInvocation for MemoryToolInvocation {
    fn get_description(&self) -> String {
        let memory_file_path = get_global_memory_file_path();
        format!("in {}", tildeify_path(&memory_file_path.to_string_lossy()))
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![ToolLocation {
            path: get_global_memory_file_path(),
            location_type: crate::core::tools::tools::LocationType::Write,
        }]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        let allowlist = self.allowlist.clone();
        let fact = self.params.fact.clone();
        let category = self.params.category.clone();
        Box::pin(async move {
            let memory_file_path = get_global_memory_file_path();
            let allowlist_key = memory_file_path.to_string_lossy().to_string();

            let already_allowed = {
                let guard = allowlist.lock().unwrap();
                guard.contains(&allowlist_key)
            };
            if already_allowed {
                return Ok(None);
            }

            let current_content = read_memory_file_content().await.map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;
            let new_content = compute_new_content(&current_content, &fact, category.as_deref());

            let file_name = memory_file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("STARCODE.md");

            let file_diff = create_patch(
                file_name,
                &current_content,
                &new_content,
                "Current",
                "Proposed",
            );

            Ok(Some(
                crate::core::tools::tools::ToolCallConfirmationDetails {
                    confirmation_type: crate::core::tools::tools::ConfirmationType::Warning,
                    title: format!(
                        "Confirm Memory Save: {}",
                        tildeify_path(&memory_file_path.to_string_lossy())
                    ),
                    prompt: file_diff,
                    on_confirm: Arc::new(|_outcome| {}),
                },
            ))
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
        let params = self.params.clone();
        Box::pin(async move {
            let SaveMemoryParams {
                fact,
                category,
                modified_by_user,
                modified_content,
            } = &params;

            if modified_by_user.unwrap_or(false) {
                if let Some(content) = modified_content {
                    let memory_file_path = get_global_memory_file_path();
                    if let Some(parent) = memory_file_path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&memory_file_path, content).await?;

                    let success_message =
                        "Okay, I've updated the memory file with your modifications.";
                    return Ok(ToolResult {
                        llm_content: Some(success_message.to_string()),
                        return_display: Some(success_message.to_string()),
                        output: success_message.to_string(),
                        error: None,
                        data: None,
                    });
                }
            }

            let memory_file_path = get_global_memory_file_path();
            if let Some(parent) = memory_file_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            let current_content = read_memory_file_content().await?;
            let new_content = compute_new_content(&current_content, fact, category.as_deref());

            tokio::fs::write(&memory_file_path, new_content).await?;

            let success_message = format!("Okay, I've remembered that: \"{}\"", fact);
            Ok(ToolResult {
                llm_content: Some(success_message.clone()),
                return_display: Some(success_message.clone()),
                output: success_message,
                error: None,
                data: None,
            })
        })
    }
}

pub struct MemoryTool {
    message_bus: Arc<MessageBus>,
    allowlist: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl MemoryTool {
    pub fn new(message_bus: Arc<MessageBus>) -> Self {
        Self {
            message_bus,
            allowlist: Arc::new(std::sync::Mutex::new(HashSet::new())),
        }
    }

    pub fn name(&self) -> &str {
        "memory"
    }

    pub fn display_name(&self) -> &str {
        "SaveMemory"
    }

    pub fn description(&self) -> &str {
        r#"Saves a specific piece of information or fact to your long-term memory.

Use this tool:

- When the user explicitly asks you to remember something (e.g., "Remember that I like pineapple on pizza", "Please save this: my cat's name is Whiskers").
- When the user states a clear, concise fact about themselves, their preferences, or their environment that seems important for you to retain for future interactions to provide a more personalized and effective assistance.

Do NOT use this tool:

- To remember conversational context that is only relevant for the current session.
- To save long, complex, or rambling pieces of text. The fact should be relatively short and to the point.
- If you are unsure whether the information is a fact worth remembering long-term. If in doubt, you can ask the user, "Should I remember that for you?"

## Parameters

- `fact` (string, required): The specific fact or piece of information to remember. This should be a clear, self-contained statement.
- `category` (string, optional): The category of the memory. Supported values: "user" (for user preferences/facts), "project" (for project-specific context). If omitted, defaults to general memory."#
    }

    pub fn kind(&self) -> Kind {
        Kind::Think
    }

    pub fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "fact": {
                    "type": "string",
                    "description": "The specific fact or piece of information to remember. Should be a clear, self-contained statement."
                },
                "category": {
                    "type": "string",
                    "description": "The category of the memory. e.g., 'user', 'project'.",
                    "enum": ["user", "project"]
                }
            },
            "required": ["fact"]
        })
    }

    pub fn validate_tool_params(&self, params: &SaveMemoryParams) -> Result<(), String> {
        if params.fact.trim().is_empty() {
            return Err("Parameter \"fact\" must be a non-empty string.".to_string());
        }
        Ok(())
    }

    pub fn build(&self, params: SaveMemoryParams) -> Box<dyn ToolInvocation> {
        Box::new(MemoryToolInvocation::new(
            params,
            self.message_bus.clone(),
            Some(self.name().to_string()),
            Some(self.display_name().to_string()),
            self.allowlist.clone(),
        ))
    }
}

impl BaseDeclarativeTool for MemoryTool {
    fn name(&self) -> &str {
        MemoryTool::name(self)
    }

    fn display_name(&self) -> &str {
        MemoryTool::display_name(self)
    }

    fn description(&self) -> &str {
        MemoryTool::description(self)
    }

    fn kind(&self) -> Kind {
        MemoryTool::kind(self)
    }

    fn parameter_schema(&self) -> serde_json::Value {
        MemoryTool::parameter_schema(self)
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: SaveMemoryParams = serde_json::from_value(params)?;
        self.validate_tool_params(&params)
            .map_err(|e| e.to_string())?;
        Ok(self.build(params))
    }
}

impl ModifiableDeclarativeTool<SaveMemoryParams> for MemoryTool {
    fn get_modify_context(&self) -> ModifyContext<SaveMemoryParams> {
        ModifyContext {
            get_file_path: Box::new(|_params: &SaveMemoryParams| {
                get_global_memory_file_path().to_string_lossy().to_string()
            }),
            get_current_content: Box::new(move |_params: &SaveMemoryParams| {
                Box::pin(async move { read_memory_file_content().await })
            }),
            get_proposed_content: Box::new(move |params: &SaveMemoryParams| {
                let fact = params.fact.clone();
                let category = params.category.clone();
                Box::pin(async move {
                    let current_content = read_memory_file_content().await?;
                    Ok(compute_new_content(
                        &current_content,
                        &fact,
                        category.as_deref(),
                    ))
                })
            }),
            create_updated_params: Box::new(
                |_old_content: String,
                 modified_proposed_content: String,
                 original_params: SaveMemoryParams| {
                    SaveMemoryParams {
                        fact: original_params.fact.clone(),
                        category: original_params.category.clone(),
                        modified_by_user: Some(true),
                        modified_content: Some(modified_proposed_content),
                    }
                },
            ),
        }
    }
}
