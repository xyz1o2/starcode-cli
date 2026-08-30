use crate::core::confirmation_bus::types::Message;
use crate::core::i18n;
use crate::runtime::messages::StreamMessage;
use tokio::sync::mpsc;

pub async fn relay_confirmation_bus_message(
    tx: &mpsc::Sender<StreamMessage>,
    message_id: u64,
    message: Message,
) {
    match message {
        Message::ToolConfirmationRequest(req) => {
            let args_value = req.tool_call.args.clone().unwrap_or(serde_json::json!({}));
            let args_json = serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string());

            // ask_user_question gets its own confirmation type with parsed options
            let mut confirmation =
                if req.tool_call.name == "ask_user_question" {
                    build_ask_user_question_confirmation(&args_value)
                } else {
                    let synthetic_tool_call = crate::types::StarToolCall {
                        id: req.correlation_id.clone(),
                        call_type: "function".to_string(),
                        function: crate::types::StarToolCallFunction {
                            name: req.tool_call.name.clone(),
                            arguments: args_json,
                        },
                    };

                    crate::ui::components::confirmation_dialog::build_confirmation_from_tool_call(
                        &synthetic_tool_call,
                    )
                    .await
                };

            if !matches!(confirmation.operation_type, crate::types::ConfirmationType::AskUserQuestion) {
                if let Some(prompt) = &req.prompt {
                    if let Some(rest) = prompt.strip_prefix("DIFF_VIEW:") {
                        let mut parts = rest.splitn(2, '\n');
                        let path = parts.next().unwrap_or("unknown").to_string();
                        let diff = parts.next().unwrap_or("").to_string();
                        confirmation.operation_type = crate::types::ConfirmationType::EditFile;
                        confirmation.details = crate::types::ConfirmationDetails::EditFile {
                            file_path: path,
                            diff,
                            old_lines: 0,
                            new_lines: 0,
                        };
                    } else if !prompt.trim().is_empty() {
                        confirmation.operation_type = crate::types::ConfirmationType::Generic;
                        confirmation.details = crate::types::ConfirmationDetails::Generic {
                            title: req.title.clone().unwrap_or_else(|| "Permission confirmation".to_string()),
                            prompt: prompt.clone(),
                        };
                        confirmation.is_dangerous = true;
                    }
                } else if let Some(title) = &req.title {
                    confirmation.operation_type = crate::types::ConfirmationType::Generic;
                    confirmation.details = crate::types::ConfirmationDetails::Generic {
                        title: title.clone(),
                        prompt: format!("是否允许执行工具 `{}`？", req.tool_call.name),
                    };
                }
            }

            confirmation.tool_name = req.tool_call.name.clone();
            let _ = tx
                .send(StreamMessage::ToolConfirmationRequest {
                    message_id,
                    tool_call_id: req.correlation_id,
                    confirmation,
                })
                .await;
        }
        Message::ToolPolicyRejection(rej) => {
            let args_value = rej.tool_call.args.clone().unwrap_or(serde_json::json!({}));
            let args_json = serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string());
            let tool_call_id = format!("policy_reject:{}:{}", message_id, rej.tool_call.name);
            let synthetic_tool_call = crate::types::StarToolCall {
                id: tool_call_id.clone(),
                call_type: "function".to_string(),
                function: crate::types::StarToolCallFunction {
                    name: rej.tool_call.name.clone(),
                    arguments: args_json,
                },
            };

            let mut confirmation =
                crate::ui::components::confirmation_dialog::build_confirmation_from_tool_call(
                    &synthetic_tool_call,
                )
                .await;
            confirmation.tool_name = rej.tool_call.name.clone();
            confirmation.outcome = Some(i18n::t(
                "ui.tool.policy_rejected",
                "Denied (policy)",
                "Denied (policy)",
            ));

            let _ = tx
                .send(StreamMessage::ToolConfirmationRequest {
                    message_id,
                    tool_call_id,
                    confirmation,
                })
                .await;
        }
        Message::ContextUpdated(ctx) => {
            let _ = tx
                .send(StreamMessage::StatsUpdate {
                    au2_compressed: true,
                    token_usage: Some(crate::types::StarUsage {
                        prompt_tokens: 0,
                        completion_tokens: 0,
                        total_tokens: ctx.new_token_count as u32,
                        ..Default::default()
                    }),
                })
                .await;
        }
        _ => {}
    }
}

/// Build a ToolConfirmation for the ask_user_question tool directly from
/// its parsed JSON arguments, bypassing the generic tool-call dialog builder.
fn build_ask_user_question_confirmation(args: &serde_json::Value) -> crate::types::ToolConfirmation {
    use crate::types::{AskUserQuestionOption, ConfirmationDetails, ConfirmationType};

    let question = args
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let header = args
        .get("header")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let multi_select = args
        .get("multiSelect")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let options: Vec<AskUserQuestionOption> = args
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|opt| {
                    AskUserQuestionOption {
                        label: opt.get("label").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        description: opt.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        preview: opt.get("preview").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    crate::types::ToolConfirmation {
        tool_name: "ask_user_question".to_string(),
        operation_type: ConfirmationType::AskUserQuestion,
        details: ConfirmationDetails::AskUserQuestion {
            question,
            header,
            options,
            multi_select,
        },
        is_dangerous: false,
        outcome: None,
    }
}
