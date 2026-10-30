//! 底部状态栏的版面：分组、按宽度取舍、最终渲染。

use super::*;

pub(super) const INTERRUPT_HINT_MIN_WIDTH: u16 = 72;

/// 中断提示文案（不含分隔符，分隔符由调用方在括号内补）。
///
/// 全行**只有这一个**中断提示。此前同一行上并存三套说法——`Esc to interrupt`、
/// 以及两处 `press Ctrl+C to cancel`——它们互斥显示，取决于"思考是否超过 20 秒"
/// 和"工具是否 stall"这类与功能无关的时间条件，等于让用户凭猜决定按哪个键。
///
/// 两个键其实都生效（`input.rs` 的 Esc 与 Ctrl+C 都发 `AgentRequest::Abort`），
/// 所以这不是能力问题而是文案问题。这里统一采用 Claude Code 的写法：小写
/// `esc`，放在行尾当 footer 提示（官方 statusline 文档把 `esc to interrupt`
/// 归在 "footer keyboard hints" 一类）。
pub(super) const INTERRUPT_HINT: &str = "esc to interrupt";

/// 这批状态文案和 spinner 动词说的是同一件事，显示出来只会变成两层废话。
/// 真正带信息量的（preflight、checkpoint、取消、卡住）不在此列。
///
/// 匹配用"整串相等"而不是前缀：`Status: running preflight checks` 以
/// `Status: running` 开头，但它恰恰是最该显示的一条。用前缀匹配会把它一起吞掉。
pub(super) const REDUNDANT_STATUS_LINES: &[&str] = &[
    "status: processing request",
    "status: processing",
    "status: starting",
    "status: ",
    "状态：处理中",
    "状态：",
];

/// 这批是**通知**，不是状态：描述的是"刚刚发生过的一件事"，而且说完就没用了。
///
/// `current_status_line` 全仓有 60 多处写入，混着两类东西——runtime 写的"现在成立"
/// 的阶段（preflight / checkpoint / 卡住）和本地 UI 对一次按键的反应
/// （`Copied` / `Theme: Dracula` / `Press Esc again to clear input` / Ctrl+O 的
/// `Verbose output: ON`）。两类都往活动 spinner 行里塞，于是通知占掉一个 `·`
/// 分隔位、与回合计时并排显示成 `18s · Verbose output: OFF`——一条过期通知
/// 伪装成"现在正在发生的事"，而且和左侧耗时读起来像同一句话的两个宾语。
///
/// 通知侧正在逐个迁往 `push_toast`（`input.rs` 的 `ToggleTranscript` 是第一处）。
/// 这里再堵一道渲染侧，是为了让约定对**尚未迁移**的写入点同样成立——只迁写入点
/// 的话，下一个写 `current_status_line` 的人会再踩一次。
///
/// 这里用**前缀**匹配，和上面 `REDUNDANT_STATUS_LINES` 的整串相等不同：这些条目的
/// 尾巴是变的（`Theme: Dracula`、`Press Ctrl+C again to exit`），要拦的是整类形状。
/// 每一条都是本地交互的回声，不存在"runtime 正在 preflight"长这样的实时状态。
pub(super) const TRANSIENT_NOTIFICATION_PREFIXES: &[&str] = &[
    "copied",
    "theme: ",
    "press ",
    "switching to model ",
    "verbose output",
];

/// 单条实时状态文案的显示上限（列）。
///
/// 这里**不再**按终端宽度做预算：装不下时由 `fit_meta_groups` 整组丢弃，让退场的
/// 是"有实时状态"这个语义，而不是半个词。此前按 `width - 24` 截断，窄屏下必然
/// 切出 `Verbose outp…` 这类残词。
///
/// 这个上限只拦病态输入（超长路径、整段报错），保证一行状态不会吃掉整行。
pub(super) const MAX_STATUS_WIDTH: usize = 40;

/// `current_status_line` 里值得显示的部分。
///
/// 这个字段全仓有 40 多处写入（runtime 的 preflight / checkpoint / 取消、
/// input 的复制与主题切换、UI 自己的 stall 提示），但在此之前**没有任何渲染消费
/// 它**——唯一能显示它的 `status_line_text` 没有调用点。于是所有「正在做什么」的
/// 提示都落进了黑洞，用户只能看到转圈和一个词。
///
/// 这一行是它天然的位置：用户盯着的就是这里。带 `Status: ` 前缀的先剥掉——前缀
/// 在这个上下文里没有信息量，真正起作用的是后面的内容。
pub(super) fn status_suffix(state: &ChatState) -> Option<String> {
    let raw = state.current_status_line.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let lowered = raw.to_lowercase();
    if REDUNDANT_STATUS_LINES.contains(&lowered.as_str()) {
        return None;
    }
    // 通知不是状态：说过即弃的东西不该占住实时状态位（见
    // TRANSIENT_NOTIFICATION_PREFIXES）。
    if TRANSIENT_NOTIFICATION_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
    {
        return None;
    }
    let stripped = raw
        .strip_prefix("Status: ")
        .or_else(|| raw.strip_prefix("Status："))
        .unwrap_or(raw)
        .trim();
    if stripped.is_empty() {
        return None;
    }
    Some(crate::ui::utils::render::truncate_to_display_width(
        stripped,
        MAX_STATUS_WIDTH,
    ))
}

/// 括号内元数据之间的分隔符。
///
/// 刻意**不用**本文件的 `SEP`（`" │ "`）：那是底部状态栏给并列字段用的，框线太重，
/// 放进括号里会把这行从"一行附注"变成"一张表格"。原来这一行用的是 `·`。
pub(super) const META_SEP: &str = " · ";

/// 按优先级把元数据装进括号里，装不下就**整组**丢弃。
///
/// `groups` 已经按优先级从高到低排好。每一组的代价是 `META_SEP` 加正文宽度；
/// 装不下的跳过而不是截断——于是窄屏退场的是"有工具名"/"有实时状态"这种语义，
/// 不会在行尾留下 `· Verbose outp…` 这样的半个单词。
///
/// 用 `continue` 而不是 `break`：低优先级的短组（`18s`）常常比高优先级但很长的组
/// （一个 30 字符的工具名）更该留下。
pub(super) fn fit_meta_groups(groups: &[(Color, String)], budget: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    for (color, text) in groups {
        let need = text.width() + META_SEP.width();
        if used + need > budget {
            continue;
        }
        if !spans.is_empty() {
            spans.push(meta_sep());
        }
        spans.push(Span::styled(text.clone(), Style::default().fg(*color)));
        used += need;
    }
    spans
}

pub(super) fn meta_sep() -> Span<'static> {
    Span::styled(META_SEP, crate::ui::themes::theme::muted_style())
}

/// 工具名在状态行里的显示上限（字符数）。
pub(super) const TOOL_NAME_MAX_CHARS: usize = 30;

/// 回合进行多久之后才开始报 token 用量（秒）。
///
/// 对标 Claude Code：30 秒之前用户还不关心上下文，数字反而挤掉动词。
pub(super) const TOKEN_DISPLAY_AFTER_SECS: u64 = 30;

/// Render a spinner line above the input area.
///
/// 这是回合期间**唯一**的实时面。同一件事此前在正文里还有一份 live 折叠头
/// （`∴ Thinking…`，primary + 粗斜体），于是"思考中"被说了两遍、两套权重、两种
/// 措辞同时在屏上。实时状态归这一行，正文只留完成后的记录
/// （`∴ Thought for 14s`）。
///
/// 构图：`⠹ 动词… (元数据 · esc to interrupt)`。
/// 动词是主角（主题强调色 + 流光），括号里全部退到 `secondary`，提示退到
/// `inactive`。元数据装不下时整组丢弃，绝不切出半个单词。
pub fn processing_spinner_line(state: &ChatState, width: u16) -> Vec<ratatui::text::Line<'static>> {
    if !state.is_processing {
        return vec![];
    }
    let elapsed = state
        .processing_started_at
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    let spinner = BRAILLE_SPINNER[state.animation_tick as usize % BRAILLE_SPINNER.len()];

    // 对标 Claude Code Spinner 的动词优先级：
    // `overrideMessage ?? currentTodo?.activeForm ?? currentTodo?.subject ?? randomVerb`。
    // 区别只在于 todo 文案要先过一遍英文/长度筛（见 in_progress_todo_verb），
    // 筛不过就用本轮固定的随机动词。
    let todo_verb = in_progress_todo_verb(state);
    let verb: &str = match todo_verb {
        Some(ref v) => v,
        None => turn_spinner_verb(state),
    };

    // 两个信号各司其职：`warmup` 是"还在算"，`stall` 是"工具可能卡住"。
    // 工具 ID 而不是显示标签才是权威来源——并行工具完成时不能把兄弟项藏掉。
    let current_tool_started = latest_active_tool_started_at(state);
    let stall = if state.is_streaming {
        tool_stall_intensity(current_tool_started, state.animation_tick)
    } else {
        0.0
    };
    let warmup = if state.is_streaming && current_tool_started.is_none() {
        warmup_intensity(state.last_token_time, state.animation_tick)
    } else {
        0.0
    };

    let theme = state.theme_manager.current();
    // spinner 字形基色取 **primary**，不是 warning。
    //
    // 此前取的是 `theme.warning`，而 13 套主题里 warning 全是 `Yellow` 或琥珀，
    // `spinner_color` 的"静默 10 秒变暖"目标色又恰好是琥珀 `Rgb(222,160,40)`
    // —— 起点即终点，那套写了三段注释论证的安抚通道在真实主题下一个像素都不会
    // 变（`a_slow_turn_warms_to_amber_and_never_to_red` 之所以是绿的，是因为它
    // 传进去的 base 是 `Rgb(10,10,10)` 而不是 `theme.warning`）。
    let spinner_color = spinner_color(theme.primary, warmup, stall, state.animation_tick);
    // 元数据一律 `secondary` 退后。此前工具名独占 `info` 蓝并自带脉动，于是全行
    // 最亮的东西既不是动词也不是时钟，而是一个 trivia。
    let meta_color = text_color(theme.secondary, stall);

    let mut spans = vec![Span::styled(
        format!(" {} ", spinner),
        Style::default().fg(spinner_color),
    )];

    // 动词是这一行的主角：主题强调色 + 全行唯一的流光。
    for (text, is_shimmer) in shimmer_segments(verb, state.animation_tick) {
        let color = if stall > 0.5 {
            lerp_color(theme.primary, theme.error, stall * 0.5)
        } else if is_shimmer {
            theme.primary_shimmer
        } else {
            theme.primary
        };
        spans.push(Span::styled(text.to_string(), Style::default().fg(color)));
    }
    spans.push(Span::styled("…", Style::default().fg(theme.primary)));

    // 元数据按优先级从高到低排。括号让它们读成"附注"而不是句子的一部分 ——
    // `Spelunking… 18s · Verbose output: OFF` 里，通知和耗时挤在同一串 `·` 里，
    // 读起来像是同一句话的两个宾语。
    let mut groups: Vec<(Color, String)> = Vec::with_capacity(5);

    // 1) 时钟：这行唯一恒定不变的东西，回答"还卡着吗"。只跟着「真报警」变色 ——
    //    回合时长本身不是异常信号，一个四分钟的构建不该因为"超过 15 秒"就变红。
    groups.push((meta_color, format_elapsed(elapsed)));

    // 2) 当前工具。静默等待确认时工具名后面不再追加任何按键提示：Esc 的用法在行尾
    //    已经写明了，再补一句 Ctrl+C 只会让人以为两者不等价。
    if let Some(tool_name) = state.current_tool_name.as_ref() {
        let chars: Vec<char> = tool_name.chars().collect();
        let short: String = chars.iter().take(TOOL_NAME_MAX_CHARS).collect();
        groups.push((
            meta_color,
            if chars.len() > TOOL_NAME_MAX_CHARS {
                format!("{short}…")
            } else {
                short
            },
        ));
    }

    // 3) 实时状态。`current_status_line` 上约有 40 处写入，其中混着两类东西：
    //    "现在成立"的状态（preflight / checkpoint / 卡住）和"刚刚发生过的通知"
    //    （Copied / Theme: X / Verbose output: ON）。两者都往这一行里塞，
    //    于是 Ctrl+O 的提示会和回合计时并排显示成 `18s · Verbose output: OFF`
    //    —— 一条过期通知占据了实时状态的位置。通知侧已迁到 `push_toast`，
    //    这里只留真正描述"现在"的那部分。
    if let Some(status) = status_suffix(state) {
        groups.push((meta_color, status));
    }

    // 4) 思考结束后的回顾标签（对标 Claude Code `thought for Xs`），驻留 2 秒
    //    后随 spinner 行一起消失。
    //
    //    这是过去时记录，不是实时声明，所以**保留**：思考正文短到不成块时
    //    （`message_render` 的 `len() > 5` 过滤），正文的 `∴ Thought for 14s`
    //    折叠头不会出现，这里是那个时长唯一的出口。
    //
    //    与之相对，"思考中"的 `· thinking 12s` 已删除：同一个"思考中"此前在屏上
    //    说了三遍（正文 `Thinking…` + 动词 + 这段计时），而删掉正文那份之后，动词
    //    已经是状态、时钟已经在走，这段既不新增信息、又把括号撑长。
    if let Some((think_secs, finished_at)) = state.thinking_finished {
        if finished_at.elapsed() < std::time::Duration::from_secs(2) {
            groups.push((meta_color, format!("thought for {}s", think_secs.max(1))));
        }
    }

    // 5) token 用量（紧凑模式）。prompt_tokens 优先，provider 只报 total_tokens
    //    时退回它——两者与 token_count 同源，不搞第二套 `↓` 格式。
    //    对标 Claude Code：30 秒之前用户还不关心上下文，数字反而挤掉动词。
    if state.token_count > 0 && elapsed >= TOKEN_DISPLAY_AFTER_SECS {
        if let Some(usage) = state.token_usage.as_ref() {
            let tokens = if usage.prompt_tokens > 0 {
                usage.prompt_tokens
            } else {
                usage.total_tokens
            };
            if tokens > 0 {
                let color = if stall > 0.5 {
                    lerp_color(theme.secondary, theme.error, stall * 0.3)
                } else {
                    theme.secondary
                };
                groups.push((color, format!("{} tokens", format_token_count(tokens))));
            }
        }
    }

    // 预算：行首缩进 1 + 字形 1 + 空格 1 + 动词 + "…" 1 + 空格 1 + "(" 1 + ")" 1
    let lead_width = 6 + verb.width();
    let show_hint = width >= INTERRUPT_HINT_MIN_WIDTH;
    let fixed = lead_width
        + 2
        + if show_hint {
            META_SEP.width() + INTERRUPT_HINT.width()
        } else {
            0
        };
    let meta = fit_meta_groups(&groups, (width as usize).saturating_sub(fixed));

    if !meta.is_empty() {
        let paren = Style::default().fg(meta_color);
        spans.push(Span::styled(" (", paren));
        spans.extend(meta);
        // 中断提示放在**行尾**。它是一次性的操作指引，不是状态；夹在状态中间会让
        // 后面还在追加的元数据跑到它后面去，行尾就不再是行尾了。Claude Code 也是
        // 把它当 footer 提示放在最后。
        if show_hint {
            spans.push(meta_sep());
            spans.push(Span::styled(
                INTERRUPT_HINT,
                Style::default().fg(theme.inactive),
            ));
        }
        spans.push(Span::styled(")", paren));
    }

    // 对标 Claude Code：spinner 行只显示动词 + 耗时，不显示模型名。
    // 模型名由底部状态栏（build_status_spans 的 "⊙ model"）负责展示。

    vec![
        ratatui::text::Line::from(""),
        ratatui::text::Line::from(spans),
    ]
}

/// Render a token usage warning line above the input area.
/// 仅在 worker 已确认真实 capacity 后显示上下文占用提醒。
pub fn token_warning_line(state: &ChatState) -> Vec<ratatui::text::Line<'static>> {
    let auto_compact_enabled = std::env::var("STAR_ENABLE_AUTO_COMPACT")
        .map(|value| !matches!(value.to_ascii_lowercase().as_str(), "0" | "false" | "off"))
        .unwrap_or(true);
    let theme = state.theme_manager.current();

    let tokens = match &state.token_usage {
        Some(usage) if usage.prompt_tokens > 0 => usage.prompt_tokens,
        Some(usage) if usage.total_tokens > 0 => usage.total_tokens,
        _ => {
            let estimated: u32 = state
                .chat_history
                .iter()
                .map(|e| {
                    (e.content.len() + e.reasoning_content.as_ref().map(|r| r.len()).unwrap_or(0))
                        as u32
                        / 4
                })
                .sum();
            if estimated == 0 {
                return vec![];
            }
            estimated
        }
    };

    let Some(policy) = confirmed_context_policy(state) else {
        return vec![];
    };
    let ctx = policy.effective_tokens;
    if ctx == 0 || tokens < policy.pre_send_threshold {
        return vec![];
    }

    let pct_used = (tokens as f64 / ctx as f64) * 100.0;
    let pct_remaining = 100.0 - pct_used;

    let (message, color) = if tokens >= policy.budget_nudge_threshold {
        if auto_compact_enabled {
            (
                format!(
                    "Context {:.0}% used · compact target {}",
                    pct_used,
                    crate::core::context_policy::format_context_window(
                        policy.compact_target_tokens
                    )
                ),
                theme.error,
            )
        } else {
            (
                format!(
                    "Context low ({:.0}% remaining) · Run /compact to compact & continue",
                    pct_remaining
                ),
                theme.error,
            )
        }
    } else if auto_compact_enabled {
        (
            format!(
                "Context {:.0}% used · compact begins near {}",
                pct_used,
                crate::core::context_policy::format_context_window(policy.compact_max_tokens)
            ),
            theme.warning,
        )
    } else {
        (
            format!("Context {:.0}% used · consider running /compact", pct_used),
            theme.warning,
        )
    };

    vec![ratatui::text::Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(message, Style::default().fg(color)),
    ])]
}

/// Return the current status text as a plain string (for full-page rendering).
pub fn status_line_text(state: &ChatState) -> String {
    if let Some(ref line) = state.current_status_line {
        line.clone()
    } else {
        let (model, pending) = status_model(state);
        let model = if model.is_empty() {
            "..."
        } else {
            model.as_str()
        };
        if pending {
            // 待生效的切换：显示用户选中的那个并标出来，别让状态栏还停在旧模型上
            // —— 那看起来就是"切了没反应"。
            format!("⊙ {} ⏳", model)
        } else {
            format!("⊙ {}", model)
        }
    }
}

pub(super) fn format_token_count(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// 状态栏显示的「上下文输入侧 token 数」。
///
/// 对标 Claude Code 的 `input + cache_creation + cache_read`，但不能照着三项相加：
/// 两种 provider 语义相反——Anthropic 的 `input_tokens` 不含缓存读，三项相加才是
/// 输入侧；OpenAI/DeepSeek 的 `prompt_tokens` 已含缓存命中（`cached_tokens` 是其
/// 子集），再相加会重复计算。唯一在两侧都成立的表达式是 `total - completion`：
/// Anthropic 侧它恰等于 `input + cache_read + cache_creation`，OpenAI 侧退回
/// `prompt_tokens`。provider 只零散上报时，回退到 prompt_tokens，最后兜 total。
pub(super) fn input_context_tokens(usage: &crate::types::StarUsage) -> u32 {
    let by_difference = usage.total_tokens.saturating_sub(usage.completion_tokens);
    if by_difference > 0 {
        by_difference
    } else if usage.prompt_tokens > 0 {
        usage.prompt_tokens
    } else {
        usage.total_tokens
    }
}

/// 为 provider 明确上报的缓存计数生成状态栏文本。
///
/// 命中率由 [`crate::types::cache_hit_rate`] 计算，用的是跨 provider 的分母
/// （输入侧总量），不是 `cache_read / prompt_tokens` —— 后者在 Anthropic 侧
/// 语义不同，会算出 >100% 的值。
pub(super) fn format_cache_telemetry(usage: Option<&crate::types::StarUsage>) -> Option<String> {
    let usage = usage.filter(|usage| usage.cache_telemetry_reported)?;
    let mut parts = Vec::with_capacity(3);
    if usage.cache_read_tokens > 0 {
        parts.push(format!(
            "read {}",
            format_token_count(usage.cache_read_tokens)
        ));
    }
    if usage.cache_creation_tokens > 0 {
        parts.push(format!(
            "write {}",
            format_token_count(usage.cache_creation_tokens)
        ));
    }
    if let Some(rate) = crate::types::cache_hit_rate(usage) {
        parts.push(format!("{rate:.0}%"));
    }
    (!parts.is_empty()).then(|| format!("Cache {}", parts.join(" · ")))
}

pub(super) fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// 本回合已经跑了多久（秒）。没有活跃回合时返回 `None`。
///
/// 只有 `is_processing` 为真才算"在跑"：`processing_started_at` 在每个回合开始时
/// 刷新，但回合结束后仍然留着旧值，直接用它会让空闲会话一直显示上一回合的耗时。
pub(super) fn turn_elapsed_secs(state: &ChatState) -> Option<u64> {
    if !state.is_processing {
        return None;
    }
    state.processing_started_at.map(|t| t.elapsed().as_secs())
}

/// Build the styled status line as a Vec<Line> for full-page rendering.
pub fn build_status_lines(state: &ChatState, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ── 模型切换确认框：必须**一直**看得见 ──────────────────────────────
    //
    // 它原先只写进 `current_status_line`，而那一行只在**回合进行中**渲染
    // （`processing_spinner_line` 在 `is_processing` 为假时返回空行）。于是空闲
    // 状态下按回车选模型 → 面板关掉 → 屏幕上什么都不出现 → 用户看到的现象就是
    // "切不动"，只能重开面板再按一次回车。
    if state.pending_model_confirmation {
        let model = state
            .pending_model_change
            .as_ref()
            .map(|pending| pending.model.clone())
            .unwrap_or_default();
        let theme = state.theme_manager.current();
        lines.push(Line::from(vec![
            Span::styled(" ⚠ ", Style::default().fg(theme.warning)),
            Span::styled(
                crate::core::i18n::t(
                    "ui.model.confirm_switch",
                    &format!("切换模型到 {}? (y/n)", model),
                    &format!("Switch model to {}? (y/n)", model),
                ),
                Style::default().fg(theme.warning),
            ),
        ]));
    }

    let spans: Vec<Span> = build_status_spans(state, width);
    if spans.is_empty() {
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(spans));
    }
    lines
}

/// 状态栏要显示的模型，以及它是否**待生效**。
///
/// worker 回执之前 `current_model` 还是旧模型。这时状态栏若照旧显示旧模型，
/// 用户看到的现象就是"选了半天没反应" —— "切不动"的观感大半来自这里，而不是
/// 切换真的没生效。所以有待生效的模型切换时显示**用户选的那个**并标上 ⏳；
/// 回执到了（生效或被拒）队列就空了，状态栏自动回到实际值。
pub fn status_model(state: &ChatState) -> (String, bool) {
    let pending = state
        .pending_runtime_settings
        .iter()
        .rev()
        .find_map(|mutation| {
            mutation
                .model
                .as_ref()
                .map(|selection| selection.model.clone())
        });
    match pending {
        Some(model) if model != state.current_model => (model, true),
        _ => (state.current_model.clone(), false),
    }
}

/// Build the styled spans for the status bar (shared by both renderers).
/// Width-aware: narrow terminals progressively hide less important sections
/// (Claude Code hides token counts / countdowns below 60 columns).
pub(super) fn build_status_spans(state: &ChatState, width: u16) -> Vec<Span<'static>> {
    let compact = width < 60;
    let minimal = width < 40;
    let mut spans: Vec<Span> = Vec::new();

    // 获取当前主题颜色
    let theme = state.theme_manager.current();

    // ── 1. Model  ⊙ model-name ──────────────────────────────────────────────
    let (model_text, model_pending) = status_model(state);
    let model = if model_text.is_empty() {
        "..."
    } else {
        &model_text
    };
    let model_color = theme.success; // 使用主题的 success 颜色
    spans.push(Span::styled(" ⊙ ", Style::default().fg(model_color)));
    spans.push(Span::styled(
        model.to_string(),
        Style::default().fg(model_color),
    ));
    if model_pending {
        // 待生效：worker 回执之前显示的是用户选的那个模型，旁边标出来 ——
        // 否则状态栏还显示旧模型，用户看到的现象就是"切了没反应"。
        spans.push(Span::styled(" ⏳", Style::default().fg(theme.warning)));
    }

    // ── 2. Directory  ▸ dirname ──────────────────────────────────────────────
    if !minimal {
        let cwd = crate::core::utils::paths::current_dir_cached();
        let dir_name = cwd
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| cwd.to_string_lossy().to_string());
        let dir_color = theme.warning; // 使用主题的 warning 颜色
        spans.push(sep());
        spans.push(Span::styled("▸ ", Style::default().fg(dir_color)));
        spans.push(Span::styled(dir_name, Style::default().fg(dir_color)));
    }

    // ── 3. Git  ⎇ branch  ↑N / ↓N / ✗ / ✓ ─────────────────────────────────
    if !minimal {
        if let Some(branch) = &state.git_branch {
            let git_color = theme.success; // 使用主题的 success 颜色
            spans.push(sep());
            spans.push(Span::styled("⎇ ", Style::default().fg(git_color)));
            spans.push(Span::styled(branch.clone(), Style::default().fg(git_color)));

            if let Some(status_str) = &state.git_status {
                let dirty = status_str.starts_with("Dirty");
                let (ahead, behind) = parse_ahead_behind(status_str);

                if dirty {
                    spans.push(Span::styled(" ✗", Style::default().fg(theme.warning)));
                }
                if let Some(a) = ahead {
                    spans.push(Span::styled(
                        format!(" ↑{}", a),
                        Style::default().fg(theme.warning_shimmer),
                    ));
                }
                if let Some(b) = behind {
                    spans.push(Span::styled(
                        format!(" ↓{}", b),
                        Style::default().fg(theme.warning),
                    ));
                }
                if !dirty && ahead.is_none() && behind.is_none() {
                    spans.push(Span::styled(" ✓", Style::default().fg(git_color)));
                }
            }
        }
    }

    // ── 4. Tokens  ⚡ 23.4k in / 5.2k out · 41.1% ────────────────────────
    if !compact {
        spans.push(sep());
        match &state.token_usage {
            Some(usage) if usage.prompt_tokens > 0 || usage.total_tokens > 0 => {
                let tokens = input_context_tokens(usage);
                if let Some(ctx) = context_window_tokens(state) {
                    let pct = (tokens as f64 / ctx as f64) * 100.0;

                    let format_tok = |n: u32| -> String {
                        if n >= 1_000_000 {
                            format!("{:.1}M", n as f64 / 1_000_000.0)
                        } else if n >= 1_000 {
                            format!("{:.1}k", n as f64 / 1_000.0)
                        } else {
                            n.to_string()
                        }
                    };

                    let token_label =
                        format!("{}/{} ({:.1}%)", format_tok(tokens), format_tok(ctx), pct);

                    let (token_color, bold) = if let Some(policy) = confirmed_context_policy(state)
                    {
                        if tokens >= policy.budget_nudge_threshold {
                            (theme.error, true)
                        } else if tokens >= policy.pre_send_threshold {
                            (theme.error_shimmer, false)
                        } else if tokens >= policy.compact_target_tokens {
                            (theme.warning_shimmer, false)
                        } else {
                            (theme.warning, false)
                        }
                    } else {
                        (theme.inactive, false)
                    };
                    let icon_style = if bold {
                        Style::default()
                            .fg(token_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(token_color)
                    };
                    spans.push(Span::styled("⚡ ", icon_style));
                    spans.push(Span::styled(token_label, Style::default().fg(token_color)));
                } else {
                    let dim = theme.inactive;
                    spans.push(Span::styled("⚡ ", Style::default().fg(dim)));
                    spans.push(Span::styled(
                        format_token_count(tokens),
                        Style::default().fg(dim),
                    ));
                }
            }
            _ => {
                // ── Fallback: 会话内尚未拿到任何 provider usage（首轮响应未结束）时，
                // 根据聊天历史估算。新回合不再清空 token_usage，故基线一旦建立，
                // 本分支在整个会话里都不会再被命中——估算与真实不再来回横跳。
                // 真正的清空只发生在 /clear（见 ui/events/input.rs），届时历史为空，
                // estimated 为 0，显示「⚡ —」。
                let estimated: u32 = state
                    .chat_history
                    .iter()
                    .map(|e| {
                        (e.content.len()
                            + e.reasoning_content.as_ref().map(|r| r.len()).unwrap_or(0))
                            as u32
                            / 4
                    })
                    .sum();
                if estimated > 0 {
                    if let Some(ctx) = context_window_tokens(state) {
                        let pct = (estimated as f64 / ctx as f64) * 100.0;
                        let format_tok = |n: u32| -> String {
                            if n >= 1_000_000 {
                                format!("~{:.1}M", n as f64 / 1_000_000.0)
                            } else if n >= 1_000 {
                                format!("~{:.1}k", n as f64 / 1_000.0)
                            } else {
                                format!("~{}", n)
                            }
                        };
                        let dim = theme.inactive;
                        spans.push(Span::styled("⚡ ", Style::default().fg(dim)));
                        spans.push(Span::styled(
                            format!(
                                "{}/{} ({:.1}%)",
                                format_tok(estimated),
                                format_tok(ctx),
                                pct
                            ),
                            Style::default().fg(dim),
                        ));
                    } else {
                        let dim = theme.inactive;
                        spans.push(Span::styled("⚡ ", Style::default().fg(dim)));
                        spans.push(Span::styled(
                            format!("~{}", format_token_count(estimated)),
                            Style::default().fg(dim),
                        ));
                    }
                } else {
                    let dim = theme.inactive;
                    spans.push(Span::styled("⚡ ", Style::default().fg(dim)));
                    spans.push(Span::styled("—", Style::default().fg(dim)));
                }
            }
        }
    }

    // ── 5. Cost + elapsed ────────────────────────────────────────────────────
    // 对标 Claude Code 状态栏的 running cost & duration。成本已由 stream.rs 累计
    // 到 state.total_cost。成本为 0（尚未产生 usage）时只显示耗时。
    //
    // 计时口径：**回合进行中显示本回合耗时，回合之间显示会话时长**。
    //
    // 此前固定显示 `session_started_at` 的差值，也就是会话年龄。那是一个跟"现在
    // 有没有在干活"毫无关系的数：用户看到它在跳，就把它当成了回合计时器，于是一
    // 个卡住的回合看起来像"还在运行"，而真正在跑的回合反倒没有任何指示 —— 界面
    // 上唯一在动的东西在说谎。
    if !compact {
        let elapsed_secs = turn_elapsed_secs(state)
            .unwrap_or_else(|| state.session_started_at.elapsed().as_secs());
        let elapsed = format_elapsed(elapsed_secs);
        if state.total_cost > 0.0 {
            let cost_label = format!("${:.2}", state.total_cost);
            spans.push(sep());
            spans.push(Span::styled(
                cost_label,
                Style::default().fg(theme.secondary),
            ));
        }
        spans.push(sep());
        spans.push(Span::styled(elapsed, Style::default().fg(theme.inactive)));
    }

    // ── 5a. Offline indicator（对标 Claude Code /network 状态指示）──────────
    if !compact && state.network_offline {
        spans.push(sep());
        spans.push(Span::styled("OFFLINE", Style::default().fg(theme.warning)));
    }

    // ── 5b. Provider cache telemetry ────────────────────────────────────────
    // Cache tokens do not share a provider-independent denominator with prompt tokens,
    // so render the raw explicitly-reported values rather than inventing a hit percentage.
    if !compact {
        if let Some(cache_label) = format_cache_telemetry(state.token_usage.as_ref()) {
            spans.push(sep());
            spans.push(Span::styled(
                cache_label,
                Style::default().fg(theme.inactive),
            ));
        }
    }

    // ── 6. Approval mode (always visible,对标 Claude Code mode indicator) ────
    {
        spans.push(sep());
        match state.approval_mode {
            ApprovalMode::Default => {
                spans.push(Span::styled(
                    "⏵⏵ default",
                    Style::default().fg(theme.subtle),
                ));
            }
            ApprovalMode::AcceptEdits => {
                spans.push(Span::styled(
                    "⏵⏵ accept-edits",
                    Style::default().fg(Color::LightGreen),
                ));
            }
            ApprovalMode::Plan => {
                spans.push(Span::styled(
                    "⏸ plan",
                    Style::default().fg(Color::Rgb(0, 102, 102)),
                ));
            }
            ApprovalMode::Yolo => {
                spans.push(Span::styled(
                    "⏵⏵ yolo",
                    Style::default()
                        .fg(theme.error)
                        .add_modifier(Modifier::BOLD),
                ));
            }
        }
    }

    // ── 6b. Thinking effort ──────────────────────────────────────────────────
    //
    // 只要模型有思考开关就一直显示，包括默认的 Off。原来是"非默认才显示"，而默认恰好
    // 是 Off —— 于是没人动过档位时状态栏、抬头、通知全是空的，用户找不到"调思考深度的
    // UI 在哪"。对标 Claude Code：档位常驻显示，只在模型不支持思考时才隐藏。
    if !compact && crate::core::config::models::supports_thinking_ui(&state.current_model) {
        spans.push(sep());
        spans.push(Span::styled(
            state.thinking_effort.label(),
            Style::default().fg(theme.secondary),
        ));
    }

    // ── 6b1. Fast mode indicator (/fast) ─────────────────────────────────────
    if state.fast_mode {
        spans.push(sep());
        spans.push(Span::styled(
            "⚡fast",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // ── 6b2. Extra working directories (/add-dir) ────────────────────────────
    if !compact && !state.extra_working_dirs.is_empty() {
        spans.push(sep());
        spans.push(Span::styled(
            format!("+{} dir", state.extra_working_dirs.len()),
            Style::default().fg(theme.info),
        ));
    }

    // ── 6b2. Vim mode indicator ──────────────────────────────────────────────
    if !compact && state.vim_enabled {
        spans.push(sep());
        let vim_label = match state.vim_state.mode {
            crate::ui::vim::VimMode::Normal => "-- NORMAL --",
            crate::ui::vim::VimMode::Insert => "-- INSERT --",
            crate::ui::vim::VimMode::Visual => "-- VISUAL --",
        };
        let vim_color = match state.vim_state.mode {
            crate::ui::vim::VimMode::Normal => theme.success,
            crate::ui::vim::VimMode::Insert => theme.info,
            crate::ui::vim::VimMode::Visual => theme.secondary,
        };
        spans.push(Span::styled(
            vim_label,
            Style::default().fg(vim_color).add_modifier(Modifier::BOLD),
        ));
    }

    // ── 6c. Compact summary indicator ────────────────────────────────────────
    if !compact && state.au2_compressed {
        spans.push(sep());
        spans.push(Span::styled(
            "[COMPACTED]",
            Style::default()
                .fg(theme.info)
                .add_modifier(Modifier::ITALIC),
        ));
    }

    // ── 7. Sandbox indicator (only when unavailable — warning state) ──────────
    if state.sandbox_enabled {
        // 探测结果缓存：Windows 上首次探测要 spawn `wsl` 子进程（300ms~2s），
        // 每帧重探测会把渲染线程拖垮。见 ChatState::sandbox_available。
        let avail = state
            .sandbox_available
            .get_or_init(crate::core::sandbox::SandboxManager::is_available);
        if !avail {
            spans.push(sep());
            spans.push(Span::styled("[!SBX]", Style::default().fg(theme.warning)));
        }
    }

    // ── 8. Tasks summary (when panel is hidden) ──────────────────────────────
    if !compact && !state.task_panel.is_visible {
        if let Some(summary) = state.task_panel.get_summary() {
            let task_color = theme.info; // 使用主题的 info 颜色
            spans.push(sep());
            spans.push(Span::styled("☐ ", Style::default().fg(task_color)));
            spans.push(Span::styled(summary, Style::default().fg(task_color)));
        }
    }

    // ── 9. UI Language indicator (only when non-default) ──────────────────────
    if !compact {
        let lang_code = crate::core::i18n::current_language().as_code();
        if lang_code != "en" {
            let lang_color = theme.secondary;
            spans.push(sep());
            spans.push(Span::styled(
                format!("🌐 {}", lang_code),
                Style::default().fg(lang_color),
            ));
        }
    }

    spans
}

pub fn render_status_bar(f: &mut Frame, state: &ChatState, area: Rect) {
    f.render_widget(Clear, area);
    // Use a block with no background to let terminal's own color show through
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );
    let spans = build_status_spans(state, area.width);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
