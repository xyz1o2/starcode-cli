use super::*;
use crate::core::confirmation_bus::types::{MessageBusType, ToolConfirmationRequest};
use crate::types::{ConfirmationDetails, ConfirmationPresentation, ConfirmationType};
use crate::ui::components::confirmation_dialog::build_confirmation_card_block;

fn request(name: &str, args: serde_json::Value) -> ToolConfirmationRequest {
    ToolConfirmationRequest {
        message_type: MessageBusType::ToolConfirmationRequest,
        tool_call: crate::core::policy::FunctionCall {
            name: name.into(),
            args: Some(args),
            read_only: None,
        },
        policy_identity: None,
        correlation_id: "approval-1".into(),
        server_name: None,
        title: None,
        prompt: None,
        reason: None,
        presentation: Default::default(),
        originating_tool_call_id: Some("call-1".into()),
    }
}

async fn relay(request: ToolConfirmationRequest) -> crate::types::ToolConfirmation {
    let (tx, mut rx) = mpsc::channel(1);
    relay_confirmation_bus_message(&tx, 7, Message::ToolConfirmationRequest(request)).await;
    let Some(StreamMessage::ToolConfirmationRequest {
        message_id,
        tool_call_id,
        originating_tool_call_id,
        confirmation,
    }) = rx.recv().await
    else {
        panic!("expected confirmation");
    };
    assert_eq!(message_id, 7);
    assert_eq!(tool_call_id, "approval-1");
    assert_eq!(originating_tool_call_id.as_deref(), Some("call-1"));
    confirmation
}

fn text(lines: &[ratatui::text::Line<'_>]) -> String {
    lines
        .iter()
        .map(crate::ui::utils::text::line_plain_text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// 复现管道测试命令：完整动作独立展示，超时诊断仅在展开时出现。
#[tokio::test]
async fn bash_warning_keeps_command_and_moves_auto_diagnostic_to_details() {
    let state = crate::ui::state::ChatState::test_fixture();
    let command = "go test ./server ./db ./agent ./config -count=1 2>&1 | tail -30";
    let mut req = request(
        "Bash",
        serde_json::json!({"command": command, "dir_path": "."}),
    );
    req.title = Some(i18n::t1("ui.confirm.shell_operators.title"));
    req.prompt = Some(i18n::t1("ui.confirm.shell_operators.warning"));
    req.reason = Some(
        i18n::t1("ui.confirm.auto_reason").replace("{reason}", "AI classifier timed out after 2s"),
    );
    req.presentation = ConfirmationPresentation {
        auto_fallback: true,
        tool_warning: req.prompt.clone(),
    };
    let confirmation = relay(req).await;
    assert!(matches!(
        confirmation.operation_type,
        ConfirmationType::ShellCommand
    ));
    assert!(
        matches!(&confirmation.details, ConfirmationDetails::ShellCommand {command: actual, working_dir, ..} if actual == command && working_dir == ".")
    );
    let theme = state.theme_manager.current();
    for width in [24, 44, 96] {
        let lines =
            build_confirmation_card_block(&confirmation, width, 1, false, false, false, "", theme);
        assert!(
            lines.iter().all(|line| line.width() <= width),
            "width={width}: {lines:?}"
        );
        let rendered = text(&lines);
        assert!(rendered.starts_with(&i18n::t1("ui.confirm.title.bash")));
        assert!(!rendered.contains("AI classifier"));
        assert!(!rendered.contains("Complex Command"));
        let reconstructed: String = lines
            .iter()
            .filter(|line| {
                line.spans
                    .first()
                    .is_some_and(|span| matches!(span.content.as_ref(), "  $ " | "    "))
            })
            .flat_map(|line| line.spans.iter().skip(1))
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(reconstructed, command, "width={width}");
        for choice in ["1. ", "2. ", "3. ", "4. "] {
            assert!(rendered.contains(choice));
        }
    }
    let expanded = text(&build_confirmation_card_block(
        &confirmation,
        96,
        1,
        true,
        false,
        false,
        "",
        theme,
    ));
    assert!(expanded.contains("AI classifier timed out after 2s"));
    let lines = build_confirmation_card_block(&confirmation, 80, 1, false, false, false, "", theme);
    crate::assert_lines_render!("bash_confirmation_action_first", 80, 18, &lines);
}

/// 显式规则的原因仍默认可见，不因为文案包含 Auto 前缀就被当作诊断隐藏。
#[tokio::test]
async fn bash_explicit_reason_is_preserved_without_text_classification() {
    let state = crate::ui::state::ChatState::test_fixture();
    let mut req = request("Bash", serde_json::json!({"command": "cargo publish"}));
    req.reason = Some("Your permission settings say `ask` for this tool — asked every time".into());
    req.title = Some("Permission required: Bash".into());
    req.prompt = Some("Explicit rule requires approval".into());
    let mut confirmation = relay(req).await;
    assert!(matches!(
        confirmation.operation_type,
        ConfirmationType::ShellCommand
    ));
    let render = |confirmation: &crate::types::ToolConfirmation| {
        text(&build_confirmation_card_block(
            confirmation,
            96,
            1,
            false,
            false,
            false,
            "",
            state.theme_manager.current(),
        ))
    };
    assert!(render(&confirmation).contains("Your permission settings say `ask`"));
    let literal =
        i18n::t1("ui.confirm.auto_reason").replace("{reason}", "a literal custom rule explanation");
    confirmation.reason = Some(literal.clone());
    assert!(render(&confirmation).contains(&literal));
}

/// 非 Bash 的正文和 DIFF_VIEW 仍走原桥接语义，不能误套命令卡。
#[tokio::test]
async fn non_bash_prompts_and_edit_diffs_keep_their_details() {
    let _state = crate::ui::state::ChatState::test_fixture();
    for name in ["SendMessage", "Read", "PluginAction"] {
        let mut req = request(name, serde_json::json!({"file_path": "src/main.rs"}));
        req.title = Some("Review request".into());
        req.prompt = Some("To: worker\nMessage: inspect all targets".into());
        let confirmation = relay(req).await;
        assert!(
            matches!(confirmation.details, ConfirmationDetails::Generic { prompt, .. } if prompt.contains("inspect all targets"))
        );
    }
    let mut req = request(
        "Edit",
        serde_json::json!({"file_path": "src/main.rs", "old_string": "a", "new_string": "b"}),
    );
    req.prompt = Some("DIFF_VIEW:src/main.rs\n-a\n+b".into());
    let confirmation = relay(req).await;
    assert!(
        matches!(confirmation.details, ConfirmationDetails::EditFile {file_path, diff, ..} if file_path == "src/main.rs" && diff == "-a\n+b")
    );
}

/// 旧会话和旧总线请求没有展示字段时仍能读取，未知来源不隐藏原始原因。
#[tokio::test]
async fn old_payloads_default_to_visible_reasons() {
    let _state = crate::ui::state::ChatState::test_fixture();
    let mut req = request("Bash", serde_json::json!({"command": "printf hello"}));
    req.reason = Some("Custom security explanation".into());
    let mut wire = serde_json::to_value(req).unwrap();
    wire.as_object_mut().unwrap().remove("presentation");
    let decoded: ToolConfirmationRequest = serde_json::from_value(wire).unwrap();
    assert!(!decoded.presentation.auto_fallback);
    let confirmation = relay(decoded).await;
    let mut wire = serde_json::to_value(confirmation).unwrap();
    wire.as_object_mut().unwrap().remove("presentation");
    let decoded: crate::types::ToolConfirmation = serde_json::from_value(wire).unwrap();
    assert!(!decoded.presentation.auto_fallback);
    assert_eq!(
        decoded.reason.as_deref(),
        Some("Custom security explanation")
    );
}
