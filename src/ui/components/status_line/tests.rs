//! 状态栏与 spinner 的测试。
//!
use super::*;

use super::*;
use crate::ui::themes::ThemeManager;

/// 把分段还原成"高亮起止列"，方便断言光带位置。
fn shimmer_columns(text: &str, tick: u64) -> Option<(usize, usize)> {
    let mut col = 0usize;
    let mut found: Option<(usize, usize)> = None;
    for (seg, is_shimmer) in shimmer_segments(text, tick) {
        let w = seg.width();
        if is_shimmer {
            found = Some((col, col + w));
        }
        col += w;
    }
    found
}

/// 模型切换的 y/n 确认框必须**一直看得见**。
///
/// 它原先只写进 `current_status_line`，而那一行只在回合进行中渲染 ——
/// 空闲时按回车选模型，面板关掉后屏幕上什么都不出现，用户看到的现象
/// 就是"切不动"。
#[test]
fn the_model_switch_confirmation_is_always_visible() {
    let mut state = crate::ui::state::ChatState::test_fixture();
    state.is_processing = false;
    state.pending_model_confirmation = true;
    state.pending_model_change = Some(crate::ui::state::store::PendingModelChange {
        model: "model-b".to_string(),
        provider_id: Some("local".to_string()),
    });

    let lines = build_status_lines(&state, 100);
    let text: String = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect();

    assert!(
        text.contains("model-b") && text.contains("(y/n)"),
        "确认框必须看得见: {:?}",
        text
    );
}

/// 没有待确认的切换时，状态栏就是原来那一行。
#[test]
fn no_confirmation_adds_no_extra_line() {
    let mut state = crate::ui::state::ChatState::test_fixture();
    state.pending_model_confirmation = false;
    let lines = build_status_lines(&state, 100);
    assert_eq!(lines.len(), 1, "不该多出一行: {:?}", lines);
}

/// 状态栏要显示用户**选中的**模型，并标出它还没生效 ——
/// worker 回执之前 `current_model` 还是旧模型，照旧显示的话用户看到的
/// 就是"切了没反应"。
#[test]
fn a_pending_model_switch_is_shown_as_pending() {
    let mut state = crate::ui::state::ChatState::test_fixture();
    state.current_model = "model-a".to_string();
    state
        .pending_runtime_settings
        .push_back(crate::runtime::messages::RuntimeSettingsMutation {
            ui_revision: 1,
            persistence: crate::runtime::messages::RuntimeSettingsPersistencePolicy::SessionOnly,
            model: Some(crate::runtime::messages::RuntimeModelSelection {
                model: "model-b".to_string(),
                provider_id: Some("local".to_string()),
            }),
            context_window: None,
            thinking_effort: None,
        });

    let (model, pending) = status_model(&state);
    assert_eq!(model, "model-b", "要显示用户选中的那个");
    assert!(pending, "并且标出还没生效");

    let text = status_line_text(&state);
    assert!(
        text.contains("model-b") && text.contains('⏳'),
        "状态栏要显示待生效的模型: {}",
        text
    );
}

/// 没有待生效的切换时，状态栏显示实际在用的模型，不带标记。
#[test]
fn an_applied_model_switch_is_shown_without_a_pending_marker() {
    let mut state = crate::ui::state::ChatState::test_fixture();
    state.current_model = "model-b".to_string();

    let (model, pending) = status_model(&state);
    assert_eq!(model, "model-b");
    assert!(!pending, "已经生效就不该再标记");
}

#[test]
fn shimmer_segments_rejoin_into_the_original_text() {
    // 三段拼回去必须和原文一字不差，否则状态行会漏字或重字
    let text = "Percolating";
    for tick in 0..(text.width() as u64 + SHIMMER_GAP_COLUMNS as u64 + 4) * SHIMMER_TICKS_PER_COLUMN
    {
        let joined: String = shimmer_segments(text, tick)
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(joined, text, "tick {} 拼接结果不一致", tick);
    }
}

#[test]
fn shimmer_band_is_narrow_and_moves() {
    // 光带宽度对标 Claude Code 的 glimmerIndex ± 1（3 列），并且要真的在走。
    // 之前的实现是 10 字符宽、从左侧长出来，短动词上整条几乎永远点亮。
    let text = "Percolating";
    let max_band = (2 * SHIMMER_BAND_HALF_WIDTH + 1) as usize;
    let mut seen = std::collections::BTreeSet::new();
    for step in 0..(text.width() + SHIMMER_GAP_COLUMNS as usize) {
        let tick = step as u64 * SHIMMER_TICKS_PER_COLUMN;
        if let Some((start, end)) = shimmer_columns(text, tick) {
            assert!(
                end - start <= max_band,
                "step {} 光带宽度 {} 超过 {}",
                step,
                end - start,
                max_band
            );
            seen.insert(start);
        }
    }
    assert!(seen.len() > 3, "光带没有移动，只出现在 {:?}", seen);
}

#[test]
fn shimmer_leaves_the_text_dark_between_sweeps() {
    // 扫光之间要有间隔：至少有一帧整条都不高亮
    let text = "Working";
    let all_dark = (0..(text.width() + SHIMMER_GAP_COLUMNS as usize))
        .any(|step| shimmer_columns(text, step as u64 * SHIMMER_TICKS_PER_COLUMN).is_none());
    assert!(all_dark, "整个周期里都有高亮，缺少扫光间隔");
}

#[test]
fn shimmer_splits_wide_characters_by_column_not_by_char() {
    // CJK 一个字占 2 列，光带宽 3 列 → 最多同时覆盖 2 个字
    let text = "正在完善前端页面";
    for step in 0..(text.width() + SHIMMER_GAP_COLUMNS as usize) {
        if let Some((start, end)) = shimmer_columns(text, step as u64 * SHIMMER_TICKS_PER_COLUMN) {
            assert!(end - start <= 4, "step {} 高亮了 {} 列", step, end - start);
        }
    }
}

#[test]
fn shimmer_handles_empty_text() {
    assert!(shimmer_segments("", 0).is_empty());
}

#[test]
fn latin_phrases_pass_the_ui_text_gate() {
    // 参照实现的动词表里有 Flambéing / Sautéing，带变音符号的拉丁词要放行
    assert!(is_latin_phrase("Running tests"));
    assert!(is_latin_phrase("Flambéing"));
    assert!(is_latin_phrase("Wiring up the /login flow"));
}

#[test]
fn wide_script_fails_the_ui_text_gate() {
    // 这条就是 bug 现场：中文 activeForm 不能进状态行
    assert!(!is_latin_phrase("正在完善前端页面"));
    assert!(!is_latin_phrase("Fixing 前端页面"));
    assert!(!is_latin_phrase("🚀 shipping"));
    // 没有字母的纯符号也不算文案
    assert!(!is_latin_phrase("---"));
    assert!(!is_latin_phrase(""));
}

#[test]
fn cache_telemetry_shows_raw_counts_and_a_cross_provider_hit_rate() {
    // 分母为 0 时不显示百分比（无法计算），只显示原始计数。
    let read_only = crate::types::StarUsage {
        cache_read_tokens: 12_345,
        cache_telemetry_reported: true,
        ..Default::default()
    };
    assert_eq!(
        format_cache_telemetry(Some(&read_only)).as_deref(),
        Some("Cache read 12.3k")
    );

    let write_only = crate::types::StarUsage {
        cache_creation_tokens: 100,
        cache_telemetry_reported: true,
        ..Default::default()
    };
    assert_eq!(
        format_cache_telemetry(Some(&write_only)).as_deref(),
        Some("Cache write 100")
    );
    assert!(format_cache_telemetry(None).is_none());
    assert!(format_cache_telemetry(Some(&crate::types::StarUsage::default())).is_none());
}

/// OpenAI 语义：`prompt_tokens` 已含缓存命中，命中率就是 cached / prompt。
#[test]
#[test]
fn cache_hit_rate_uses_input_side_denominator_for_openai_semantics() {
    let usage = crate::types::StarUsage {
        prompt_tokens: 10_000,
        completion_tokens: 100,
        total_tokens: 10_100,
        cache_read_tokens: 9_000,
        cache_creation_tokens: 0,
        cache_telemetry_reported: true,
    };
    let rate = crate::types::cache_hit_rate(&usage).expect("rate should be computed");
    assert!((rate - 90.0).abs() < 0.1, "got {rate}");
    assert_eq!(
        format_cache_telemetry(Some(&usage)).as_deref(),
        Some("Cache read 9.0k · 90%")
    );
}

/// Anthropic 语义：`input_tokens` 不含缓存读，输入侧总量是
/// `total - completion`，绝不能拿 `prompt_tokens` 当分母（会算出 >100%）。
#[test]
#[test]
fn cache_hit_rate_does_not_exceed_100_for_anthropic_semantics() {
    let usage = crate::types::StarUsage {
        prompt_tokens: 1_000,
        completion_tokens: 500,
        total_tokens: 10_500,
        cache_read_tokens: 9_000,
        cache_creation_tokens: 500,
        cache_telemetry_reported: true,
    };
    let rate = crate::types::cache_hit_rate(&usage).expect("rate should be computed");
    assert!(
        rate <= 100.0,
        "Anthropic 语义下命中率不得超过 100%，得到 {rate}"
    );
    assert!((rate - 90.0).abs() < 0.1, "got {rate}");
}

/// provider 没上报缓存遥测时不该编一个命中率出来。
#[test]
#[test]
fn cache_hit_rate_is_none_without_reported_telemetry() {
    let usage = crate::types::StarUsage {
        prompt_tokens: 10_000,
        total_tokens: 10_100,
        completion_tokens: 100,
        cache_read_tokens: 9_000,
        cache_creation_tokens: 0,
        cache_telemetry_reported: false,
    };
    assert!(crate::types::cache_hit_rate(&usage).is_none());
}

#[test]
fn input_context_tokens_counts_anthropic_cache_read_and_creation() {
    // Anthropic：input_tokens 不含缓存读，total = input + read + creation + output。
    // 上下文占用必须三者相加，只取 prompt_tokens 会系统性低估。
    let anthropic = crate::types::StarUsage {
        prompt_tokens: 8_000,
        completion_tokens: 1_200,
        total_tokens: 8_000 + 40_000 + 500 + 1_200,
        cache_read_tokens: 40_000,
        cache_creation_tokens: 500,
        cache_telemetry_reported: true,
    };
    assert_eq!(input_context_tokens(&anthropic), 48_500);
}

#[test]
fn input_context_tokens_does_not_double_count_openai_cached_tokens() {
    // OpenAI/DeepSeek：prompt_tokens 已含缓存命中，cached_tokens 是其子集。
    // 仍然三项相加会把同一批 token 算两遍，必须走 total - completion。
    let openai = crate::types::StarUsage {
        prompt_tokens: 50_000,
        completion_tokens: 900,
        total_tokens: 50_900,
        cache_read_tokens: 45_000,
        cache_creation_tokens: 0,
        cache_telemetry_reported: true,
    };
    assert_eq!(input_context_tokens(&openai), 50_000);
}

#[test]
fn input_context_tokens_falls_back_when_total_omits_input() {
    // 只零散上报的 provider：total 与 completion 相等时回退 prompt_tokens，
    // 全空时兜底 total_tokens，绝不让状态栏凭空归零。
    let partial = crate::types::StarUsage {
        prompt_tokens: 7_000,
        completion_tokens: 7_000,
        total_tokens: 7_000,
        ..Default::default()
    };
    assert_eq!(input_context_tokens(&partial), 7_000);

    let total_only = crate::types::StarUsage {
        total_tokens: 333,
        ..Default::default()
    };
    assert_eq!(input_context_tokens(&total_only), 333);
}

/// 回归：变暖通道必须真的有地方可走。
///
/// `spinner_color` 的基色此前取 `theme.warning`，而 13 套主题里 warning 全是
/// `Yellow` 或琥珀，变暖目标色又恰好是琥珀 `Rgb(222,160,40)` —— 起点即终点，
/// "静默 10 秒变暖"在任何真实主题下都不会产生任何可见变化。
///
/// 上一条 `a_slow_turn_warms_to_amber_and_never_to_red` 之所以是绿的，是因为它
/// 喂进去的 base 是 `Rgb(10,10,10)`，绕开了这个 bug。这里直接拿真实主题值当
/// 基色断言"确实变了"，免得它再回来一次。
#[test]
fn warmup_actually_moves_the_spinner_in_a_real_theme() {
    let manager = ThemeManager::new();
    let theme = manager.current();

    // 冷态与暖态必须可区分。暖态取 40 秒静默（满格）。
    let cold = spinner_color(theme.primary, 0.0, 0.0, 0);
    let warm = spinner_color(theme.primary, 1.0, 0.0, 0);
    assert_ne!(
        cold, warm,
        "spinner 必须真的从基色走到琥珀；两者相同说明变暖通道断了"
    );
    assert_eq!(warm, Color::Rgb(222, 160, 40));
}

/// 回归：`current_status_line` 里的**通知**不该出现在活动 spinner 行上。
///
/// 该字段上混着两类东西："现在成立"的状态，和"刚刚发生过的通知"
/// （`Copied` / `Theme: X` / **Verbose output: ON**）。两类都往这一行里塞，
/// 于是 Ctrl+O 的提示会占住一个 `·` 分隔位、与回合计时并排显示成
/// `18s · Verbose output: OFF` —— 一条过期通知伪装成了"现在正在发生的事"，
/// 而且和左侧的耗时读起来像同一句话的两个宾语。
///
/// 通知侧已迁到 `push_toast`（见 `input.rs` 的 `ToggleTranscript`）；这里守的是
/// 渲染侧那道拦网，好让尚未迁移的写入点（`Copied`、`Theme:`、
/// `Press Esc again to …`）同样不漏。真状态必须照常显示。
#[test]
fn notifications_never_leak_into_the_live_line() {
    let mut state = ChatState::new();
    state.is_processing = true;
    state.processing_started_at =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(18));

    // 对照：真状态要显示。
    state.current_status_line = Some("Status: running preflight checks".to_string());
    let live: String = processing_spinner_line(&state, 120)[1]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(live.contains("running preflight checks"), "{live:?}");

    // 通知类文案：一条都不许漏进来。
    for notification in [
        "Verbose output: OFF",
        "Copied",
        "Theme: Dracula",
        "Press Esc again to clear input",
        "Press Ctrl+C again to exit",
        "Switching to model gpt-4o…",
    ] {
        state.current_status_line = Some(notification.to_string());
        let line: String = processing_spinner_line(&state, 120)[1]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !line.contains(notification),
            "通知 {notification:?} 又漏回活动行了：{line:?}"
        );
    }
}

/// 回归：元数据装不下时**整组**丢弃，不切出半个单词。
///
/// 此前是逐段 `width - 24` 截断，窄屏必然切出 `Verbose outp…` 这种残词。
/// 分组之后退场的是"有工具名"/"有实时状态"这种语义。
#[test]
fn narrow_terminals_drop_whole_meta_groups_never_half_a_word() {
    let mut state = ChatState::new();
    state.is_processing = true;
    state.processing_started_at =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(64));
    state.current_tool_name = Some("Bash".to_string());
    state.current_status_line = Some("Status: running preflight checks".to_string());

    // 宽屏：两段都在。
    let wide = processing_spinner_line(&state, 120);
    let wide_text: String = wide[1].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(wide_text.contains("Bash"), "{wide_text:?}");
    assert!(wide_text.contains("preflight"), "{wide_text:?}");

    // 逐级收窄：任何一段都不许出现被切断的残词，且任何宽度下都不许溢出。
    let status = "running preflight checks";
    for width in 40..120u16 {
        let line = processing_spinner_line(&state, width);
        let text: String = line[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.width() <= width as usize,
            "{width} 列下渲染了 {} 列，会溢出：{text:?}",
            text.width()
        );
        // 出现即整段：` · ` 分隔符 + 完整文案。绝不会是半截。
        let present = text.contains(status);
        assert_eq!(
            present,
            text.contains(&format!(" · {status}")),
            "{width} 列下实时状态只该整段在或整段不在：{text:?}"
        );
        assert!(
            !text.contains("..."),
            "{width} 列下不该出现旧的逐段截断标记：{text:?}"
        );
        // 动词和时钟在任何宽度下都必须还在
        assert!(
            text.contains("Combobulating") || text.contains('…'),
            "{width} 列下动词不该消失：{text:?}"
        );
        assert!(text.contains("1m 4s"), "{width} 列下时钟不该消失：{text:?}");
    }
}

#[test]
fn active_tool_timestamp_does_not_depend_on_display_label() {
    let mut state = ChatState::new();
    let older = std::time::Instant::now() - std::time::Duration::from_secs(20);
    let newer = std::time::Instant::now() - std::time::Duration::from_secs(5);
    state.tool_started_at.insert("older".to_string(), older);
    state.tool_started_at.insert("newer".to_string(), newer);

    assert!(state.current_tool_name.is_none());
    assert_eq!(latest_active_tool_started_at(&state), Some(newer));
}

#[test]
fn interrupt_hint_requires_a_wide_active_request() {
    let mut state = ChatState::new();
    assert!(processing_spinner_line(&state, 120).is_empty());

    state.is_processing = true;
    let wide: String = processing_spinner_line(&state, INTERRUPT_HINT_MIN_WIDTH)[1]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(wide.contains("esc to interrupt"));

    // 窄终端只丢提示，动词和耗时必须还在。
    let narrow: String = processing_spinner_line(&state, INTERRUPT_HINT_MIN_WIDTH - 1)[1]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(!narrow.contains("esc to interrupt"));
    assert!(
        narrow.contains('…'),
        "窄终端不该把动词和耗时一起丢掉：{narrow:?}"
    );
}

/// 回归：状态栏那个秒数在回合进行中必须是**本回合**耗时。
///
/// 它此前固定显示会话年龄（`session_started_at`），也就是"这个进程开了多久"。
/// 那是一个跟"现在有没有在干活"毫无关系的数，却是界面上唯一一直在动的东西：
/// 用户自然把它读成回合计时器 —— 于是"任务还在运行中"和"什么都没有发生"这两
/// 种真实状态，在界面上长得一模一样。
#[test]
fn elapsed_counter_tracks_the_turn_while_processing() {
    let mut state = ChatState::new();
    // 会话已经开了很久，但这一回合刚开始。
    state.session_started_at = std::time::Instant::now() - std::time::Duration::from_secs(3600);
    state.processing_started_at =
        Some(std::time::Instant::now() - std::time::Duration::from_secs(7));

    assert_eq!(turn_elapsed_secs(&state), None, "空闲时没有回合计时");

    state.is_processing = true;
    assert_eq!(
        turn_elapsed_secs(&state),
        Some(7),
        "回合进行中必须报本回合耗时，而不是一小时前的会话年龄"
    );

    // 回合结束：回合计时让位给会话年龄，不能把 7s 继续挂在那里。
    state.is_processing = false;
    assert_eq!(turn_elapsed_secs(&state), None);
}

/// 回归：整行只允许存在**一个**中断提示，且措辞统一为 `esc to interrupt`。
///
/// 修复前同一行上并存三套说法——`Esc to interrupt` 和两处
/// `press Ctrl+C to cancel`——它们互斥显示，切换条件是"思考是否超过 20 秒"
/// 和"工具是否 stall"，与功能无关。两个键其实都发 `AgentRequest::Abort`，
/// 所以用户实际只需要知道一个。
#[test]
fn spinner_line_carries_one_interrupt_hint_in_one_wording() {
    for (label, tool, thinking) in [
        ("idle", None, false),
        ("thinking", None, true),
        ("running tool", Some("Bash"), false),
        ("stalled tool", Some("Bash"), true),
    ] {
        let mut state = ChatState::new();
        state.is_processing = true;
        state.current_tool_name = tool.map(|t| t.to_string());
        if thinking {
            state.thinking_started_at =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(300));
        }
        if let Some(name) = tool {
            state.tool_started_at.insert(
                name.to_string(),
                std::time::Instant::now() - std::time::Duration::from_secs(90),
            );
        }

        let text: String = processing_spinner_line(&state, 120)[1]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();

        assert_eq!(
            text.matches("esc to interrupt").count(),
            1,
            "{label}: 中断提示必须恰好出现一次，实际 {text:?}"
        );
        assert!(
            !text.contains("Ctrl+C"),
            "{label}: 不该再出现 Ctrl+C 提示，实际 {text:?}"
        );
        assert!(
            !text.contains("Esc to interrupt"),
            "{label}: 大写 Esc 的旧措辞应已废弃，实际 {text:?}"
        );
    }
}

/// 回归：长时间静默是"还在算"，不是"卡住了"。
///
/// 此前 stall 判定 3 秒起跳、5 秒插值到满红，并附带一句
/// `press Ctrl+C to cancel`。思考段落之间静默几十秒是常态，首 token 等一分钟
/// 更常见，于是整轮几乎全程红字 —— 红色成了默认状态，也就不再携带信息，
/// 用户既分不清"在跑"和"卡死"，真卡死时也不会当回事。
///
/// 对标 Claude Code v2.1.138 changelog："the spinner now warms to amber after
/// 10 seconds to signal Claude is still working"。关键是**琥珀**，且只染
/// spinner 字形，不染文字。
#[test]
fn a_slow_turn_warms_to_amber_and_never_to_red() {
    let base = Color::Rgb(10, 10, 10);
    const WARM: Color = Color::Rgb(222, 160, 40);
    const STALLED: Color = Color::Rgb(171, 43, 63);

    // 未到阈值：保持主题色。
    assert_eq!(
        warmup_intensity(
            Some(std::time::Instant::now() - std::time::Duration::from_secs(5)),
            0,
        ),
        0.0
    );

    // 稳态插值点上是纯琥珀色。
    let warming = warmup_intensity(
        Some(std::time::Instant::now() - std::time::Duration::from_secs(40)),
        0,
    );
    assert!(warming > 0.9, "40 秒静默应接近满格，实际 {warming}");
    assert_eq!(spinner_color(base, warming, 0.0, 0), WARM);

    // 静默 5 分钟依然是琥珀，不是红：变暖强度会饱和，不会继续往红走。
    let long_wait = warmup_intensity(
        Some(std::time::Instant::now() - std::time::Duration::from_secs(300)),
        0,
    );
    assert!(long_wait > 0.9, "变暖强度应饱和：{long_wait}");
    assert_eq!(spinner_color(base, long_wait, 0.0, 0), WARM);
    assert_ne!(spinner_color(base, long_wait, 0.0, 0), STALLED);

    // 文字 span 完全不参与变暖，只在工具真报警时偏红。
    assert_eq!(text_color(base, 0.0), base);
    assert_eq!(text_color(base, 1.0), text_color(base, 1.0));
    let alarmed_text = text_color(base, 1.0);
    assert_ne!(alarmed_text, base, "工具报警时文字也应当偏红");
    assert_ne!(
        alarmed_text, STALLED,
        "文字只跟着偏移 70%，不与 spinner 字形一样染满红 —— 整行全红会盖掉动词本身"
    );
}

/// 反向保证：工具超时仍然是真报警，且压过变暖。
#[test]
fn a_tool_that_never_finishes_still_turns_red() {
    let base = Color::Rgb(10, 10, 10);
    const STALLED: Color = Color::Rgb(171, 43, 63);

    let running = tool_stall_intensity(
        Some(std::time::Instant::now() - std::time::Duration::from_secs(5)),
        0,
    );
    assert_eq!(running, 0.0, "工具刚起步不该报警");

    let overdue = tool_stall_intensity(
        Some(std::time::Instant::now() - std::time::Duration::from_secs(120)),
        0,
    );
    assert!(
        overdue > 0.9,
        "120 秒未结束的工具应满格报警，实际 {overdue}"
    );
    // 工具报警优先于变暖：正在跑一个四分钟的构建恰恰最该让人知道它在占资源。
    assert_eq!(spinner_color(base, 1.0, overdue, 0), STALLED);
}

/// 回归：`current_status_line` 此前全仓 40+ 处写入、零处渲染。
///
/// 它是这一行天然的位置——用户盯着的就是这里。现在带信息量的状态（preflight、
/// checkpoint、取消、卡住）会显示出来，而与动词重复的 `Status: processing`
/// 一类直接吞掉，否则这行会变成两层废话。
#[test]
fn current_status_line_reaches_the_spinner_row() {
    let mut state = ChatState::new();
    state.current_status_line = Some("Status: running preflight checks".to_string());

    // 不处理时这一行整体缺席，状态不该被塞进别处。
    assert!(processing_spinner_line(&state, 120).is_empty());

    state.is_processing = true;
    let text: String = processing_spinner_line(&state, 120)[1]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        text.contains("running preflight checks"),
        "状态必须真的显示出来，实际 {text:?}"
    );
    assert!(
        !text.contains("Status: "),
        "前缀在这个上下文里没有信息量，应当剥掉，实际 {text:?}"
    );

    // 与动词重复的泛化状态不显示。
    state.current_status_line = Some("Status: processing request".to_string());
    let text: String = processing_spinner_line(&state, 120)[1]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(
        !text.to_lowercase().contains("processing"),
        "与动词重复的状态应当吞掉，实际 {text:?}"
    );
}

/// 回归：动词表换成 Claude Code 真实的那份之后，池子里不再有闲置/字面任务词。
///
/// 旧表 200 条里塞了 16 个 Napping/Snoozing/Loafing 类的闲置动词和 13 个
/// Reading/Compiling/Indexing 类的字面任务词（后者是在编造一个并不存在的具体
/// 动作），还有 14 条重复把 `hash % len` 的分布打歪。
#[test]
fn verb_table_matches_the_real_claude_code_list() {
    const REMOVED: &[&str] = &[
        "Napping",
        "Snoozing",
        "Dozing",
        "Dreaming",
        "Lazing",
        "Loafing",
        "Lounging",
        "Lolling",
        "Resting",
        "Reclining",
        "Sprawling",
        "Futzing",
        "Dawdling",
        "Pottering",
        "Twiddling",
        "Reading",
        "Writing",
        "Compiling",
        "Executing",
        "Searching",
        "Indexing",
        "Debugging",
        "Planning",
        "Reasoning",
        "Validating",
        "Optimizing",
        "Analyzing",
    ];
    for verb in REMOVED {
        assert!(!SPINNER_VERBS.contains(verb), "`{verb}` 不该出现在动词表里");
    }

    // `Thinking` 是唯一在任何时刻都不会说错的词，必须在。
    assert!(
        SPINNER_VERBS.contains(&"Thinking"),
        "Thinking 必须留在动词表里"
    );

    // 不允许重复项：重复会按出现次数加权，打歪 `hash % len` 的分布。
    let mut seen = std::collections::HashSet::new();
    for verb in SPINNER_VERBS {
        assert!(seen.insert(*verb), "`{verb}` 重复了");
    }

    assert_eq!(SPINNER_VERBS.len(), 187, "应与 Claude Code 真实表等长");
}
