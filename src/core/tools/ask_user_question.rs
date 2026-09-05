use crate::core::config::Config;
use crate::core::confirmation_bus::MessageBus;
use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolInvocation, ToolLocation,
    ToolResult,
};
use crate::types::AskUserQuestionOption;
use crate::types::ToolConfirmationOutcome;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskUserQuestionParams {
    pub question: String,
    #[serde(default)]
    pub header: Option<String>,
    pub options: Vec<AskUserQuestionOption>,
    #[serde(rename = "multiSelect")]
    #[serde(default)]
    pub multi_select: bool,
}

pub struct AskUserQuestionTool {
    message_bus: Arc<MessageBus>,
}

/// Pre-process ask_user_question params to handle LLM passing options as JSON string.
/// Some LLMs pass `options` as a JSON string like `"[{\"label\": \"Yes\"}]"` instead
/// of an actual array. This function detects and converts such cases.
fn preprocess_ask_user_params(
    params: serde_json::Value,
) -> Result<AskUserQuestionParams, Box<dyn std::error::Error + Send + Sync>> {
    let mut params = params;

    // If options is a string, try to parse it as JSON
    if let Some(options) = params.get("options") {
        if options.is_string() {
            if let Some(options_str) = options.as_str() {
                match serde_json::from_str::<serde_json::Value>(options_str) {
                    Ok(parsed) => {
                        params["options"] = parsed;
                    }
                    Err(_) => {
                        // Try to parse as comma-separated labels
                        let labels: Vec<serde_json::Value> = options_str
                            .split(',')
                            .map(|s| {
                                let label = s.trim().trim_matches('"').trim();
                                serde_json::json!({
                                    "label": label,
                                    "description": label
                                })
                            })
                            .collect();
                        params["options"] = serde_json::Value::Array(labels);
                    }
                }
            }
        }
    }

    Ok(serde_json::from_value(params)?)
}

impl AskUserQuestionTool {
    pub fn new(_config: Arc<Config>, message_bus: Arc<MessageBus>) -> Self {
        Self { message_bus }
    }
}

impl BaseDeclarativeTool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "ask_user_question"
    }

    fn display_name(&self) -> &str {
        "Ask User Question"
    }

    fn description(&self) -> &str {
        "Asks the user a question with a set of predefined options. Use this when you need to gather user preferences, clarify ambiguous instructions, or get decisions on implementation choices. The user can select one or more options (multiSelect: true). Users will always be able to select 'Other' to provide custom text input."
    }

    fn kind(&self) -> Kind {
        Kind::Other
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user."
                },
                "header": {
                    "type": "string",
                    "description": "A header text to display above the question."
                },
                "options": {
                    "type": "array",
                    "description": "A list of predefined options for the user to choose from.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "A short label for this option (e.g., 'Fix all errors')."
                            },
                            "description": {
                                "type": "string",
                                "description": "A longer description of what this option means."
                            },
                            "preview": {
                                "type": "string",
                                "description": "Optional markdown content to preview when this option is focused."
                            }
                        },
                        "required": ["label", "description"]
                    }
                },
                "multiSelect": {
                    "type": "boolean",
                    "description": "If true, the user can select multiple options. Defaults to false (single selection)."
                }
            },
            "required": ["question", "options"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        // Pre-process params to handle LLM passing options as JSON string instead of array
        let params = preprocess_ask_user_params(params)?;
        Ok(Box::new(AskUserQuestionInvocation {
            params,
            message_bus: self.message_bus.clone(),
            outcome: Arc::new(Mutex::new(None)),
        }))
    }
}

pub struct AskUserQuestionInvocation {
    params: AskUserQuestionParams,
    message_bus: Arc<MessageBus>,
    outcome: Arc<Mutex<Option<ToolConfirmationOutcome>>>,
}

impl AskUserQuestionInvocation {
    fn format_options_for_display(&self) -> String {
        let mut output = String::new();
        for (i, option) in self.params.options.iter().enumerate() {
            output.push_str(&format!(
                "  {}. **{}** — {}\n",
                i + 1,
                option.label,
                option.description
            ));
        }
        if self.params.multi_select {
            output.push_str("\n(You may select multiple options)\n");
        }
        output
    }

    fn build_prompt(&self) -> String {
        let mut prompt = String::new();

        if let Some(ref header) = self.params.header {
            prompt.push_str(&format!("# {}\n\n", header));
        }

        prompt.push_str(&format!("**{}**\n\n", self.params.question));
        prompt.push_str(&self.format_options_for_display());

        prompt
    }
}

impl ToolInvocation for AskUserQuestionInvocation {
    fn get_description(&self) -> String {
        if let Some(ref header) = self.params.header {
            format!("Ask User: {}", header)
        } else {
            format!(
                "Ask User: {}",
                crate::utils::string_utils::truncate_with_ellipsis(&self.params.question, 60)
            )
        }
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
        let prompt = self.build_prompt();
        let title = self
            .params
            .header
            .clone()
            .unwrap_or_else(|| "Question".to_string());
        let outcome = self.outcome.clone();
        let selection_mode = if self.params.multi_select {
            "多选"
        } else {
            "单选"
        };

        Box::pin(async move {
            Ok(Some(ToolCallConfirmationDetails {
                confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                title: format!("{} ({})", title, selection_mode),
                prompt,
                on_confirm: std::sync::Arc::new(move |confirmation_outcome| {
                    if let Ok(mut stored) = outcome.lock() {
                        *stored = Some(confirmation_outcome);
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
        let question = self.params.question.clone();
        let header = self.params.header.clone();
        let options = self.params.options.clone();
        let multi_select = self.params.multi_select;
        let outcome = self.outcome.clone();

        Box::pin(async move {
            let stored = outcome.lock().map_err(|e| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )) as Box<dyn std::error::Error>
            })?;

            match stored.as_ref() {
                Some(ToolConfirmationOutcome::UserAnswer { answers, text_input }) => {
                    // User selected specific options — return them to LLM
                    let mut llm_content = String::new();
                    if let Some(ref h) = header {
                        llm_content.push_str(&format!("## {}\n\n", h));
                    }
                    llm_content.push_str(&format!(
                        "User was asked: **{}**\n\n",
                        question
                    ));

                    if let Some(ref text) = text_input {
                        if !text.is_empty() {
                            llm_content.push_str(&format!("User provided custom input: {}\n\n", text));
                        }
                    }

                    if !answers.is_empty() {
                        llm_content.push_str("User selected:\n");
                        for answer in answers {
                            llm_content.push_str(&format!("- **{}**\n", answer));
                        }
                    }

                    let display = if let Some(ref text) = text_input {
                        if answers.is_empty() {
                            format!("User input: {}", text)
                        } else if answers.len() == 1 {
                            format!("User selected: {} (with input: {})", answers[0], text)
                        } else {
                            format!("User selected {} options (with input: {})", answers.len(), text)
                        }
                    } else if answers.is_empty() {
                        "User submitted no selection.".to_string()
                    } else if answers.len() == 1 {
                        format!("User selected: {}", answers[0])
                    } else {
                        format!("User selected {} options: {}", answers.len(), answers.join(", "))
                    };

                    Ok(ToolResult {
                        llm_content: Some(llm_content),
                        return_display: Some(display.clone()),
                        output: display,
                        error: None,
                        data: None,
                    })
                }
                Some(ToolConfirmationOutcome::Cancel) => {
                    Ok(ToolResult {
                        llm_content: Some("User cancelled the question. Please continue execution without obtaining additional preference information, or ask again later.".to_string()),
                        return_display: Some("User cancelled the question.".to_string()),
                        output: "User cancelled the question.".to_string(),
                        error: None,
                        data: None,
                    })
                }
                // Legacy confirmation outcomes (ProceedOnce, etc.) — treat as confirmed
                // but without specific answer data
                Some(_) => {
                    let mut llm_content = String::new();
                    if let Some(ref h) = header {
                        llm_content.push_str(&format!("## {}\n\n", h));
                    }
                    llm_content.push_str(&format!(
                        "User was asked: **{}**\n\n",
                        question
                    ));
                    llm_content.push_str("Available options:\n");
                    for (i, opt) in options.iter().enumerate() {
                        llm_content.push_str(&format!(
                            "{}. **{}** — {}\n",
                            i + 1,
                            opt.label,
                            opt.description
                        ));
                    }
                    if multi_select {
                        llm_content.push_str("\n(Multiple selections allowed)\n");
                    }
                    llm_content.push_str("\nPlease wait for the user to reply with their choice in the conversation, then continue execution based on the user's choice.");

                    Ok(ToolResult {
                        llm_content: Some(llm_content),
                        return_display: Some("Question submitted to user, waiting for reply.".to_string()),
                        output: "Question presented to user. Waiting for response.".to_string(),
                        error: None,
                        data: None,
                    })
                }
                None => {
                    Ok(ToolResult {
                        llm_content: Some("User did not respond to the question. Please continue without the user's input.".to_string()),
                        return_display: Some("No response from user.".to_string()),
                        output: "No response from user.".to_string(),
                        error: None,
                        data: None,
                    })
                }
            }
        })
    }
}
