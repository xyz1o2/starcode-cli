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
            let mut confirmation = if req.tool_call.name == "AskUserQuestion" {
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

            if !matches!(
                confirmation.operation_type,
                crate::types::ConfirmationType::AskUserQuestion
            ) {
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
                            title: req
                                .title
                                .clone()
                                .unwrap_or_else(|| "Permission confirmation".to_string()),
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
            // 「这次为什么要问」的一行说明 —— 由 pass_gate 按卡片来源填，
            // 只透传不加工。工具自带的卡为 None（它自己的 prompt 已解释）。
            confirmation.reason = req.reason.clone();
            let _ = tx
                .send(StreamMessage::ToolConfirmationRequest {
                    message_id,
                    tool_call_id: req.correlation_id,
                    originating_tool_call_id: req.originating_tool_call_id.clone(),
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
                    originating_tool_call_id: None,
                    confirmation,
                })
                .await;
        }
        Message::ContextUpdated(_ctx) => {
            let _ = tx
                .send(StreamMessage::StatsUpdate {
                    au2_compressed: true,
                    // Compression reports a context estimate, not provider usage.
                    token_usage: None,
                })
                .await;
        }
        _ => {}
    }
}

/// Build a ToolConfirmation for the ask_user_question tool directly from
/// its parsed JSON arguments, bypassing the generic tool-call dialog builder.
///
/// 支持两种入参形态（对标 Claude Code）：
/// - `questions` 数组：每项含 `question` / `header` / `options` / `multiSelect`；
/// - 旧形态：顶层单个 `question` + `options` + `multiSelect`，包成单问题数组。
///
/// 两种形态里 `options` 都可能被 LLM 写成 JSON 字符串或逗号分隔的
/// label 串，走 [`parse_options_value`] 统一容错。
fn build_ask_user_question_confirmation(
    args: &serde_json::Value,
) -> crate::types::ToolConfirmation {
    use crate::types::{AskUserQuestionItem, ConfirmationDetails, ConfirmationType};

    // 单个原始问题 JSON → 规范化的 AskUserQuestionItem。
    let to_item = |raw: &serde_json::Value| {
        let question = raw
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let header = raw
            .get("header")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let multi_select = raw
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        AskUserQuestionItem {
            question,
            header,
            options: raw
                .get("options")
                .map(parse_options_value)
                .unwrap_or_default(),
            multi_select,
        }
    };

    let questions: Vec<AskUserQuestionItem> = match args.get("questions") {
        // questions 数组形态——每个元素独立解析。
        Some(serde_json::Value::Array(arr)) => arr.iter().map(to_item).collect(),
        // 单问题形态（兼容旧调用，也是绝大多数调用的现实路径）。
        _ => vec![to_item(args)],
    };

    crate::types::ToolConfirmation {
        tool_name: "AskUserQuestion".to_string(),
        operation_type: ConfirmationType::AskUserQuestion,
        details: ConfirmationDetails::AskUserQuestion { questions },
        is_dangerous: false,
        outcome: None,
        reason: None,
    }
}

/// 解析 AskUserQuestion 的 `options` 字段值，容忍三种写法：
/// 1. 正常的对象数组；
/// 2. JSON 字符串（部分 LLM 会把数组序列化成字符串再塞回来）；
/// 3. 逗号分隔的 label 串（如 `"Yes, No"`）——每个 label 生成一个选项。
pub(crate) fn parse_options_value(
    raw: &serde_json::Value,
) -> Vec<crate::types::AskUserQuestionOption> {
    use crate::types::AskUserQuestionOption;

    match raw {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|opt| AskUserQuestionOption {
                label: opt
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                description: opt
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                preview: opt
                    .get("preview")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string()),
            })
            .collect(),
        serde_json::Value::String(s) => {
            let s = s.trim();
            if (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('{') && s.ends_with('}'))
            {
                serde_json::from_str::<serde_json::Value>(s)
                    .ok()
                    .map(|parsed| parse_options_value(&parsed))
                    .unwrap_or_default()
            } else {
                s.split(',')
                    .map(|label| {
                        let label = label.trim().trim_matches('"').trim();
                        AskUserQuestionOption {
                            label: label.to_string(),
                            description: label.to_string(),
                            preview: None,
                        }
                    })
                    .collect()
            }
        }
        _ => Vec::new(),
    }
}
