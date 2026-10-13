//! `inspect` 的呈现：把旋钮目录与只读会话上下文渲染成模型能直接据此决策的表格。
//!
//! 呈现要解决两个具体问题：
//! 1. **模型凭什么调？** —— 只列旋钮名和值，模型无从判断该往哪调。所以每行都带一句
//!    `summary`，说明"调了会发生什么"。
//! 2. **调完什么时候生效？** —— 这是最容易产生误解的地方。同一批旋钮里既有
//!    立即生效的，也有下一轮迭代才生效的；`effect` 列如实标注，避免模型以为
//!    改完当前这次工具调用就变了。

use super::knobs::{knobs, Knob};
use super::overrides::{resolve, ValueSource};

/// 只读的会话上下文。与旋钮表分开：这些项要么在构造期冻结、要么由别的协议
/// （`RuntimeSettingsMutation`）负责，给模型一个写了不生效的写入面比不给更糟。
#[derive(Clone, Debug, Default)]
pub struct SessionContext {
    pub model: String,
    pub provider_id: Option<String>,
    pub context_window: usize,
    pub context_window_source: String,
    pub thinking_effort: String,
    pub registered_tools: usize,
}

/// 渲染完整的自调视图。
pub fn render(session: &SessionContext) -> String {
    let mut out = String::new();
    out.push_str("## Self-tuning knobs\n\n");
    out.push_str(&render_budget());
    out.push_str(
        "These are live runtime settings you can change yourself. Effective value is resolved as \
         `self_tuned` > `env` > `default`, so values you set here win over environment variables \
         without disturbing them.\n\n",
    );
    out.push_str("| knob | value | source | effect | default | env var |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- |\n");

    for knob in knobs() {
        out.push_str(&render_row(knob));
    }

    out.push_str(
        "\nEffect column: `immediate` = the next read inside the current run sees it; \
                  `next_loop_iteration` = the current run picks it up on its next turn. Neither \
                  needs a restart.\n",
    );

    let locked = super::overrides::locked_knob_keys();
    if !locked.is_empty() {
        out.push_str(&format!(
            "\nLocked by an operator — do not attempt to change these: {}.\n",
            locked
                .iter()
                .map(|key| format!("`{key}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    out.push_str("\n## Session context (read-only)\n\n");
    out.push_str(&render_session(session));
    out
}

/// 预算必须在模型动手**之前**可见。
///
/// 这是对标 ToolSelf 的 reconfiguration budget：它限制的是"还能调几次"，因此
/// 模型必须能预先据此决定"这次要不要调"。调完才告诉预算没了，就只剩下盲调。
fn render_budget() -> String {
    format!(
        "Change budget for this session: **{} of {} self-tuning changes left.** Spend it on \
         changes you can justify from something you observed, not on trying values. `reset` is \
         always free and never runs out, so you can always undo what you did.\n\n",
        super::overrides::remaining_budget(),
        super::overrides::DEFAULT_SESSION_BUDGET,
    )
}

fn render_row(knob: &Knob) -> String {
    let (value, source) = resolve(knob.key);
    let effective = value.unwrap_or_else(|| {
        knob.default
            .map(|d| d.to_string())
            .unwrap_or_else(|| "(per-caller)".to_string())
    });
    // Default 来源且没有静态默认值时，说明调用点会自选，值列已经写了 per-caller。
    let source = if matches!(source, ValueSource::Default) && knob.default.is_none() {
        "per-caller"
    } else {
        source.as_str()
    };
    let marker = if super::overrides::is_locked(knob.key) {
        " (locked)"
    } else {
        ""
    };
    format!(
        "| `{}`{} | `{}` | {} | {} | {} | `{}` |\n",
        knob.key,
        marker,
        effective,
        source,
        knob.effect.as_str(),
        knob.default.unwrap_or("(per-caller)"),
        knob.env,
    )
}

fn render_session(session: &SessionContext) -> String {
    let provider = session.provider_id.as_deref().unwrap_or("(default)");
    format!(
        "- model: `{}` (provider: `{}`)\n- context window: {} tokens (source: {})\n\
         - thinking effort: `{}`\n- tools registered: {}\n\n\
         Model and context window are frozen into the per-message agent at turn start, so they \
         are not in the knob table: change them with `/model` and `/context` and they apply to \
         the next user message, not to the turn in flight.\n",
        session.model,
        provider,
        session.context_window,
        session.context_window_source,
        session.thinking_effort,
        session.registered_tools,
    )
}

/// 渲染 `set` / `reset` 的结果。
///
/// 必须报告**改前值**：模型据此判断自己是否真的改变了什么，而不是把一次
/// 无变化的写入当成成功。
pub fn render_change(
    action: &str,
    keys: &[String],
    changes: &[(String, Option<String>, Option<String>)],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {action} applied\n\n"));
    if changes.is_empty() {
        out.push_str("No knob was overridden before or after; nothing changed.\n");
        return out;
    }
    for (key, before, after) in changes {
        let before = before.as_deref().unwrap_or("(unset)");
        let after = after
            .as_deref()
            .unwrap_or("(unset — falls back to env or default)");
        out.push_str(&format!("- `{key}`: `{before}` → `{after}`\n"));
    }
    if keys.is_empty() {
        return out;
    }
    out.push_str(&format!(
        "\nAffected: {}. Effect timing is per-knob; see `inspect`.\n",
        keys.join(", ")
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionContext {
        SessionContext {
            model: "test-model".into(),
            provider_id: Some("test-provider".into()),
            context_window: 200_000,
            context_window_source: "configured default".into(),
            thinking_effort: "medium".into(),
            registered_tools: 7,
        }
    }

    #[test]
    fn render_lists_every_knob_with_its_env_var() {
        let out = render(&session());
        for knob in knobs() {
            assert!(
                out.contains(&format!("| `{}` |", knob.key)),
                "knob {} missing from report:\n{}",
                knob.key,
                out
            );
            assert!(
                out.contains(knob.env),
                "env var {} missing:\n{}",
                knob.env,
                out
            );
        }
    }

    #[test]
    fn render_explains_effect_timing_and_read_only_scope() {
        let out = render(&session());
        assert!(out.contains("Neither needs a restart"), "{out}");
        assert!(
            out.contains("/model"),
            "read-only items must name the real path:\n{out}"
        );
        assert!(out.contains("not to the turn in flight"), "{out}");
    }

    #[test]
    fn render_states_the_budget_before_the_model_spends_it() {
        let out = render(&session());
        assert!(out.contains("Change budget for this session"), "{out}");
        assert!(
            out.contains("reset` is always free and never runs out"),
            "the undo path must be advertised up front:\n{out}"
        );
    }

    #[test]
    fn render_shows_session_context() {
        let out = render(&session());
        assert!(out.contains("`test-model`"));
        assert!(out.contains("200000 tokens"));
        assert!(out.contains("`medium`"));
        assert!(out.contains("tools registered: 7"));
    }

    #[test]
    fn render_change_reports_before_and_after() {
        let out = render_change(
            "set",
            &["compact.max_tokens".to_string()],
            &[(
                "compact.max_tokens".into(),
                Some("150000".into()),
                Some("80000".into()),
            )],
        );
        assert!(out.contains("`150000` → `80000`"), "{out}");
    }

    #[test]
    fn render_change_says_so_when_nothing_moved() {
        let out = render_change("reset", &[], &[]);
        assert!(out.contains("nothing changed"), "{out}");
    }

    #[test]
    fn unset_per_caller_knob_is_labelled_not_blank() {
        let out = render(&session());
        assert!(out.contains("(per-caller)"), "{out}");
    }
}
