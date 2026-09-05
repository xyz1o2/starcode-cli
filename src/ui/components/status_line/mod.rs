use crate::{types::ApprovalMode, ui::state::ChatState};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub mod git;

const SEP: &str = " │ ";

fn sep() -> Span<'static> {
    Span::styled(SEP, Style::default().fg(Color::DarkGray))
}

/// Known context window sizes for popular models.
/// Used when the API doesn't return context_window in /models.
fn known_context_window(model: &str) -> Option<u32> {
    let lower = model.to_lowercase();

    // Gemini 1M+ context
    if lower.contains("gemini-2.5-pro") || lower.contains("gemini-2.5-flash") {
        return Some(1_048_576);
    }
    if lower.contains("gemini-2.0-flash") {
        return Some(1_048_576);
    }
    if lower.contains("gemini-1.5-pro") || lower.contains("gemini-1.5-flash") {
        return Some(1_048_576);
    }
    if lower.contains("gemini-pro") {
        return Some(1_048_576);
    }

    // GPT-4.1 series — 1M context
    if lower.contains("gpt-4.1") {
        return Some(1_047_576);
    }

    // GPT-4o / GPT-4-turbo — 128k
    if lower.contains("gpt-4o") || lower.contains("gpt-4-turbo") {
        return Some(128_000);
    }

    // OpenAI reasoning models — 200k
    if lower.starts_with("o1") || lower.starts_with("o3") || lower.starts_with("o4") {
        return Some(200_000);
    }

    // Claude — 200k
    if lower.contains("claude") {
        return Some(200_000);
    }

    // DeepSeek — 128k
    if lower.contains("deepseek") {
        return Some(128_000);
    }

    // Qwen
    if lower.contains("qwen-max") || lower.contains("qwen-plus") || lower.contains("qwen-turbo") {
        return Some(131_072);
    }
    if lower.contains("qwen") {
        return Some(131_072);
    }

    // Llama
    if lower.contains("llama-4") {
        return Some(1_048_576);
    }
    if lower.contains("llama") {
        return Some(128_000);
    }

    // Mistral
    if lower.contains("mistral-large") {
        return Some(128_000);
    }
    if lower.contains("mistral") {
        return Some(128_000);
    }

    // Grok
    if lower.contains("grok") {
        return Some(131_072);
    }

    None
}

fn context_window_tokens(state: &ChatState) -> u32 {
    // 0. User override takes highest priority
    if let Some(override_val) = state.context_window_override {
        return override_val;
    }
    // 1. 模型专用上下文窗口：从 API /models 缓存中查
    if !state.current_model.is_empty() {
        if let Some(ctx) =
            crate::agent::model_catalog::get_cached_context_window(&state.current_model)
        {
            return ctx;
        }
    }
    // 2. 按模型名匹配已知上下文窗口
    if !state.current_model.is_empty() {
        if let Some(ctx) = known_context_window(&state.current_model) {
            return ctx;
        }
    }
    // 3. 环境变量
    std::env::var("STAR_CONTEXT_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(128_000u32)
}

/// Parse "ahead N" and "behind N" counts from a git status summary string.
/// Input examples: "Clean ahead 1", "Dirty ahead 2, behind 1", "Clean"
fn parse_ahead_behind(s: &str) -> (Option<u32>, Option<u32>) {
    let ahead = parse_count_after(s, "ahead");
    let behind = parse_count_after(s, "behind");
    (ahead, behind)
}

fn parse_count_after(s: &str, keyword: &str) -> Option<u32> {
    let idx = s.find(keyword)?;
    let rest = s[idx + keyword.len()..].trim_start();
    rest.split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
}

/// Claude Code style spinner characters - 使用固定宽度字符避免行移动
/// ● 和 ○ 同属 East Asian Ambiguous 宽度类，在任何终端下宽度一致；
/// 不能用空格 —— CJK 终端下 ● 是双宽、空格是单宽，交替会导致行内文字左右移动。
const SPINNER_FRAMES: &[&str] = &["●", "○", "●", "○"];
const SPINNER_FRAMES_REV: &[&str] = &["○", "●", "○", "●"];

/// Braille spinner frames for smoother animation
const BRAILLE_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Random verbs matching Claude Code's spinner variety.
/// 184 whimsical verbs for the thinking spinner.
const SPINNER_VERBS: &[&str] = &[
    // Core
    "Working",
    "Processing",
    "Computing",
    "Analyzing",
    "Reasoning",
    "Generating",
    "Planning",
    "Searching",
    "Reading",
    "Writing",
    "Compiling",
    "Executing",
    "Validating",
    "Optimizing",
    "Indexing",
    // Whimsical (Claude Code style)
    "Cogitating",
    "Percolating",
    "Shenaniganing",
    "Moonwalking",
    "Ruminating",
    "Pondering",
    "Musing",
    "Meditating",
    "Contemplating",
    "Deliberating",
    "Pondering",
    "Reflecting",
    "Ruminating",
    "Brooding",
    "Considering",
    "Weighing",
    "Evaluating",
    "Assessing",
    "Investigating",
    "Exploring",
    "Discovering",
    "Unraveling",
    "Deciphering",
    "Decoding",
    "Interpreting",
    "Synthesizing",
    "Composing",
    "Constructing",
    "Assembling",
    "Crafting",
    "Forging",
    "Fabricating",
    "Manufacturing",
    "Producing",
    "Creating",
    "Conjuring",
    "Manifesting",
    "Materializing",
    "Crystallizing",
    "Precipitating",
    "Fermenting",
    "Brewing",
    "Distilling",
    "Refining",
    "Polishing",
    "Honing",
    "Sharpening",
    "Tempering",
    "Annealing",
    "Forging",
    "Tinkering",
    "Fiddling",
    "Tweaking",
    "Adjusting",
    "Calibrating",
    "Balancing",
    "Harmonizing",
    "Orchestrating",
    "Conducting",
    "Choreographing",
    "Waltzing",
    "Tangoing",
    "Sambaing",
    "Jiving",
    "Grooving",
    "Vibing",
    "Flowing",
    "Gliding",
    "Soaring",
    "Floating",
    "Drifting",
    "Wandering",
    "Meandering",
    "Rambling",
    "Roaming",
    "Adventuring",
    "Exploring",
    "Discovering",
    "Uncovering",
    "Revealing",
    "Illuminating",
    "Enlightening",
    "Educating",
    "Informing",
    "Enlightening",
    "Debugging",
    "Squashing",
    "Stomping",
    "Eradicating",
    "Eliminating",
    "Vanquishing",
    "Conquering",
    "Defeating",
    "Overcoming",
    "Surmounting",
    "Transcending",
    "Surpassing",
    "Excelling",
    "Thriving",
    "Flourishing",
    "Blossoming",
    "Blooming",
    "Flowering",
    "Sprouting",
    "Growing",
    "Evolving",
    "Transforming",
    "Metamorphosing",
    "Transmuting",
    "Alchemizing",
    "Transmogrifying",
    "Shape-shifting",
    "Morphing",
    "Adapting",
    "Adjusting",
    "Calibrating",
    "Tuning",
    "Synchronizing",
    "Harmonizing",
    "Balancing",
    "Orchestrating",
    "Conducting",
    "Directing",
    "Guiding",
    "Steering",
    "Navigating",
    "Piloting",
    "Captain-ing",
    "Commanding",
    "Leading",
    "Following",
    "Trailing",
    "Tracking",
    "Hunting",
    "Stalking",
    "Chasing",
    "Pursuing",
    "Following",
    "Tailgating",
    "Shadowing",
    "Lurking",
    "Skulking",
    "Creeping",
    "Sneaking",
    "Sidling",
    "Ambling",
    "Sauntering",
    "Strolling",
    "Promenading",
    "Parading",
    "Marching",
    "Striding",
    "Swaggering",
    "Strutting",
    "Prancing",
    "Cavorting",
    "Frolicking",
    "Gamboling",
    "Romping",
    "Playing",
    "Fiddling",
    "Twiddling",
    "Futzing",
    "Puttering",
    "Pottering",
    "Dawdling",
    "Dilly-dallying",
    "Lollygagging",
    "Loafing",
    "Lazing",
    "Lounging",
    "Lolling",
    "Sprawling",
    "Reclining",
    "Resting",
    "Napping",
    "Snoozing",
    "Dozing",
    "Dreaming",
    "Fantasizing",
    "Imagining",
    "Envisioning",
    "Visualizing",
    "Picturing",
    "Conceiving",
];

/// 返回一个随机动词（用于 Claude Code 风格的 thinking spinner）
pub fn random_spinner_verb() -> &'static str {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .hash(&mut hasher);
    let idx = hasher.finish() as usize % SPINNER_VERBS.len();
    SPINNER_VERBS[idx]
}

/// Stall detection threshold (seconds)
const STALL_THRESHOLD_SECS: u64 = 3;
/// Stall transition duration (seconds)
const STALL_TRANSITION_SECS: u64 = 2;

/// Get elapsed time color: green < 5s, yellow < 15s, red >= 15s
fn elapsed_color(secs: u64) -> Color {
    if secs < 5 {
        Color::Rgb(80, 220, 100) // Green
    } else if secs < 15 {
        Color::Rgb(255, 200, 50) // Yellow
    } else {
        Color::Rgb(255, 80, 80) // Red
    }
}

/// Generate a pulse opacity effect based on animation tick (returns 0.0-1.0 range mapped to color intensity)
fn pulse_color(base: Color, tick: u64, speed: u64) -> Color {
    let phase = (tick / speed) % 6;
    let intensity = match phase {
        0 => 0.7,
        1 => 0.85,
        2 => 1.0,
        3 => 1.0,
        4 => 0.85,
        _ => 0.7,
    };
    match base {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * intensity) as u8,
            (g as f64 * intensity) as u8,
            (b as f64 * intensity) as u8,
        ),
        _ => base,
    }
}

/// Interpolate between two colors based on factor (0.0 = a, 1.0 = b)
fn lerp_color(a: Color, b: Color, factor: f64) -> Color {
    let factor = factor.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => Color::Rgb(
            (ar as f64 + (br as f64 - ar as f64) * factor) as u8,
            (ag as f64 + (bg as f64 - ag as f64) * factor) as u8,
            (ab as f64 + (bb as f64 - ab as f64) * factor) as u8,
        ),
        _ => b,
    }
}

/// Build a progress bar string for long operations (shows elapsed / expected)
fn progress_bar_str(tick: u64, width: usize) -> String {
    let pos = (tick as usize / 2) % (width * 2);
    let actual_pos = if pos >= width { width * 2 - pos } else { pos };
    let mut bar = String::with_capacity(width);
    for i in 0..width {
        if i == actual_pos {
            bar.push('█');
        } else if i == actual_pos.saturating_sub(1) || i == actual_pos + 1 {
            bar.push('▓');
        } else {
            bar.push('░');
        }
    }
    bar
}

/// Calculate stall intensity (0.0 = normal, 1.0 = fully stalled)
fn stall_intensity(
    is_streaming: bool,
    last_token_time: Option<std::time::Instant>,
    has_active_tools: bool,
    tool_started_at: Option<std::time::Instant>,
    animation_tick: u64,
) -> f64 {
    if !is_streaming {
        return 0.0;
    }

    // LLM stream stall detection (no active tools)
    if !has_active_tools {
        let Some(last_time) = last_token_time else {
            return 0.0;
        };
        let elapsed = last_time.elapsed().as_secs();
        if elapsed < STALL_THRESHOLD_SECS {
            return 0.0;
        }
        let stall_progress =
            ((elapsed - STALL_THRESHOLD_SECS) as f64 / STALL_TRANSITION_SECS as f64).min(1.0);
        let pulse = (animation_tick as f64 * 0.1).sin() * 0.1;
        return (stall_progress + pulse).clamp(0.0, 1.0);
    }

    // Tool execution stall detection (tool running too long)
    if let Some(started) = tool_started_at {
        let elapsed = started.elapsed().as_secs();
        // Warn after 30 seconds, fully stalled after 60 seconds
        if elapsed >= 30 {
            let stall_progress = ((elapsed - 30) as f64 / 30.0).min(1.0);
            let pulse = (animation_tick as f64 * 0.15).sin() * 0.1;
            return (stall_progress + pulse).clamp(0.0, 1.0);
        }
    }

    0.0
}

/// Get spinner color with stall detection
fn spinner_color_with_stall(base_color: Color, stall: f64, animation_tick: u64) -> Color {
    let stalled_color = Color::Rgb(171, 43, 63); // ERROR_RED from Claude Code
    if stall > 0.0 {
        lerp_color(base_color, stalled_color, stall)
    } else {
        pulse_color(base_color, animation_tick, 3)
    }
}

/// 流光光带的半宽（列）：光带总宽 `2 * HALF + 1`，对标 Claude Code 的
/// `glimmerIndex ± 1`（GlimmerMessage.tsx）。
const SHIMMER_BAND_HALF_WIDTH: i64 = 1;

/// 两次扫光之间的间隔（列）：周期取 `文本宽度 + GAP`，光带走到文本之外的
/// 那段时间就是间隔。对标 Claude Code 的 `cycleLength = width + 20`。
const SHIMMER_GAP_COLUMNS: i64 = 20;

/// 光带每前进一列要经过的动画帧数。
///
/// `animation_tick` 每绘制一帧 +1，帧间隔 33–50ms（见 runtime.rs 的
/// target_framerate），4 帧 ≈ 165–200ms，对上 Claude Code 的 200ms
/// （`glimmerSpeed`，50ms 时钟下每 4 帧走一列）。
const SHIMMER_TICKS_PER_COLUMN: u64 = 4;

/// Render a shimmer effect on a text string.
/// Returns a vector of (text, is_shimmer) segments.
///
/// 对标 Claude Code 的 GlimmerMessage：高亮是一条 3 列宽的窄光带，从右向左
/// 扫过文字，扫出去之后停一会儿再从右边进来。
///
/// 按**显示列**切分而不是按字符数：CJK 字符占 2 列，按字符数算光带在中文里
/// 会走得比看起来快一倍；更要紧的是，之前这里的光带宽 10 个字符，遇到
/// 「正在完善前端页面」这种 8 字短语时整条几乎永远被点亮 —— 看上去就是不动。
fn shimmer_segments(text: &str, animation_tick: u64) -> Vec<(&str, bool)> {
    let total_width = text.width() as i64;
    if total_width == 0 {
        return vec![];
    }

    // 光带中心从 total_width + GAP/2 递减到负值，走完一圈重新开始
    let cycle_len = total_width + SHIMMER_GAP_COLUMNS;
    let step = (animation_tick / SHIMMER_TICKS_PER_COLUMN) as i64;
    let center = total_width + SHIMMER_GAP_COLUMNS / 2 - step.rem_euclid(cycle_len);
    let band_start = center - SHIMMER_BAND_HALF_WIDTH;
    let band_end = center + SHIMMER_BAND_HALF_WIDTH;

    // 光带完全在文本之外：整条按暗色渲染（这就是两次扫光之间的间隔）
    if band_start >= total_width || band_end < 0 {
        return vec![(text, false)];
    }

    // 按列位置把字符分到 before / shimmer / after 三段。col 单调递增，
    // 所以两个字节游标只需各自向前推进。
    let mut col = 0i64;
    let mut before_end = 0usize;
    let mut shimmer_end = 0usize;
    for (byte_idx, ch) in text.char_indices() {
        let ch_width = ch.width().unwrap_or(0) as i64;
        let ch_end = byte_idx + ch.len_utf8();
        if col + ch_width <= band_start {
            before_end = ch_end;
            shimmer_end = ch_end;
        } else if col <= band_end {
            shimmer_end = ch_end;
        }
        col += ch_width;
    }

    let mut segments = Vec::with_capacity(3);
    if before_end > 0 {
        segments.push((&text[..before_end], false));
    }
    if shimmer_end > before_end {
        segments.push((&text[before_end..shimmer_end], true));
    }
    if shimmer_end < text.len() {
        segments.push((&text[shimmer_end..], false));
    }

    segments
}

/// spinner 动词的最大显示宽度（列）。
///
/// Claude Code 靠 SpinnerAnimationRow 的逐段宽度门控来保护右侧的耗时/token
/// 后缀，我们这行没有门控，所以在源头卡一刀：太长的 todo 文案不进 spinner。
const MAX_TODO_VERB_WIDTH: usize = 32;

/// 本轮对话固定使用的随机动词。
///
/// 对标 Claude Code：动词在 spinner 挂载时抽一次就定下来
/// （`const [randomVerb] = useState(() => sample(getSpinnerVerbs()))`），整轮
/// 不再变 —— 那一行的动感来自文字上扫过的流光，不是来自换词。之前这里写的是
/// `SPINNER_VERBS[elapsed / 5 % len]`，每 5 秒跳一个词，反而不像参照实现。
///
/// 这里没有"挂载"可挂，就拿每轮递增的 `active_message_id` 当种子：轮内稳定，
/// 换轮换词。哈希一遍是为了让相邻两轮不至于按 SPINNER_VERBS 的顺序往下念。
fn turn_spinner_verb(state: &ChatState) -> &'static str {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    state.active_message_id.unwrap_or(0).hash(&mut hasher);
    SPINNER_VERBS[hasher.finish() as usize % SPINNER_VERBS.len()]
}

/// 这段文字能不能当界面文案用。
///
/// 判据是"每个字符都不超过 1 列，且至少有一个 ASCII 字母"：宽字符（CJK、
/// emoji）直接排除，`Flambéing` 这种带变音符号的拉丁词照收 —— 参照实现的
/// 动词表里就有。
fn is_latin_phrase(text: &str) -> bool {
    text.chars().all(|c| c.width().unwrap_or(0) <= 1)
        && text.chars().any(|c| c.is_ascii_alphabetic())
}

/// 取进行中 todo 的进行时描述当 spinner 动词，取不到就返回 None。
///
/// Claude Code 是无条件用的（`currentTodo?.activeForm ?? currentTodo?.subject`），
/// 因为它的 activeForm 被 schema 约束成英文现在进行时（"Running tests"）。我们的
/// 模型跟着对话语言走，中文会话里写出来的是「正在完善前端页面」—— 直接塞进状态行
/// 就成了一句钉死的中文，既违反"界面文案用英文"，也让这行看起来卡住了。所以只收
/// 英文短语，其余回落到动词池。
fn in_progress_todo_verb(state: &ChatState) -> Option<String> {
    use crate::core::tasks::models::TaskStatus;

    let graph = &state.task_panel.task_manager.graph;
    // 按 root_ids 的顺序找第一个进行中的任务，和清单里看到的顺序一致
    // （对标 `tasksV2.find(...)`）。root_ids 里没有就退回到全表扫描，
    // 保证子任务也能命中。
    let node = graph
        .root_ids
        .iter()
        .filter_map(|id| graph.nodes.get(id))
        .find(|n| n.status == TaskStatus::InProgress)
        .or_else(|| {
            graph
                .nodes
                .values()
                .filter(|n| n.status == TaskStatus::InProgress)
                .min_by_key(|n| (n.created_at, n.id.clone()))
        })?;

    let text = node
        .active_form
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| node.title.trim());

    // 模型有时自己带了省略号，而这行后面还会再补一个「… 」
    let text = text.trim_end_matches(['…', '.', ' ']);
    if text.is_empty() || !is_latin_phrase(text) || text.width() > MAX_TODO_VERB_WIDTH {
        return None;
    }
    Some(text.to_string())
}

/// Render a spinner line above the input area.
/// Only shows when processing WITHOUT active thinking (thinking block handles itself).
pub fn processing_spinner_line(state: &ChatState) -> Vec<ratatui::text::Line<'static>> {
    if !state.is_processing {
        return vec![];
    }
    let elapsed = state
        .processing_started_at
        .map(|t| t.elapsed().as_secs())
        .unwrap_or(0);

    // Claude Code style spinner: forward + reverse cycle
    let frame_idx = state.animation_tick as usize;
    let total_frames = SPINNER_FRAMES.len() + SPINNER_FRAMES_REV.len();
    let cycle_frame = frame_idx % total_frames;
    let spinner = if cycle_frame < SPINNER_FRAMES.len() {
        SPINNER_FRAMES[cycle_frame]
    } else {
        SPINNER_FRAMES_REV[cycle_frame - SPINNER_FRAMES.len()]
    };

    // 对标 Claude Code Spinner 的动词优先级：
    // `overrideMessage ?? currentTodo?.activeForm ?? currentTodo?.subject ?? randomVerb`。
    // 区别只在于 todo 文案要先过一遍英文/长度筛（见 in_progress_todo_verb），
    // 筛不过就用本轮固定的随机动词。
    let todo_verb = in_progress_todo_verb(state);
    let verb: &str = match todo_verb {
        Some(ref v) => v,
        None => turn_spinner_verb(state),
    };
    let e_color = elapsed_color(elapsed);

    // Stall detection with smooth transition
    // Get the start time of the current active tool (if any)
    let current_tool_started = state.current_tool_name.as_ref().and_then(|_| {
        // Find the most recent tool start time
        state.tool_started_at.values().max().copied()
    });
    let stall = stall_intensity(
        state.is_streaming,
        state.last_token_time,
        state.current_tool_name.is_some(),
        current_tool_started,
        state.animation_tick,
    );

    // 获取当前主题颜色
    let theme = state.theme_manager.current();

    // Spinner color with stall detection
    let base_spinner_color = theme.warning; // 使用主题的 warning 颜色作为 spinner 基色
    let spinner_color = spinner_color_with_stall(base_spinner_color, stall, state.animation_tick);

    // Message color dims when stalled (smooth transition)
    let base_msg_color = theme.secondary;
    let stalled_msg_color = theme.error;
    let message_color = if stall > 0.0 {
        lerp_color(base_msg_color, stalled_msg_color, stall * 0.7)
    } else {
        base_msg_color
    };

    // Shimmer effect on verb text (Claude Code style)
    let shimmer_color = theme.secondary_shimmer; // 使用主题的 shimmer 颜色
    let dim_color = theme.secondary;

    let mut spans = vec![Span::styled(
        format!(" {} ", spinner),
        Style::default().fg(spinner_color),
    )];

    // Render verb with shimmer effect
    let shimmer_seg = shimmer_segments(verb, state.animation_tick);
    for (text, is_shimmer) in shimmer_seg {
        let color = if stall > 0.5 {
            lerp_color(dim_color, stalled_msg_color, stall * 0.5)
        } else if is_shimmer {
            shimmer_color
        } else {
            dim_color
        };
        spans.push(Span::styled(text.to_string(), Style::default().fg(color)));
    }
    spans.push(Span::styled("… ", Style::default().fg(dim_color)));

    // Elapsed time with smooth color transition
    let e_color_smooth = if stall > 0.0 {
        lerp_color(e_color, theme.error, stall * 0.5)
    } else {
        e_color
    };
    spans.push(Span::styled(
        format_elapsed(elapsed),
        Style::default().fg(e_color_smooth),
    ));

    // Show thinking state with sine wave opacity
    if let Some(thinking_start) = state.thinking_started_at {
        let thinking_secs = thinking_start.elapsed().as_secs();
        // Sine wave opacity for thinking indicator (Claude Code style)
        let thinking_opacity = ((state.animation_tick as f64 * 0.05).sin() + 1.0) / 2.0;
        // Change color to red when thinking exceeds 20 seconds
        let thinking_color = if thinking_secs > 20 {
            lerp_color(theme.error, theme.error_shimmer, thinking_opacity)
        } else {
            lerp_color(theme.secondary, theme.secondary_shimmer, thinking_opacity)
        };
        let thinking_label = if thinking_secs > 20 {
            format!(
                " · thinking {} (press Ctrl+C to cancel)",
                format_elapsed(thinking_secs)
            )
        } else {
            format!(" · thinking {}", format_elapsed(thinking_secs))
        };
        spans.push(Span::styled(
            thinking_label,
            Style::default().fg(thinking_color),
        ));
    }

    // 对标 Claude Code：spinner 行只显示动词 + 耗时，不显示模型名。
    // 模型名由底部状态栏（build_status_spans 的 "⊙ model"）负责展示。

    // Token count with smooth increment effect
    // (Claude Code only shows the token counter after 30s of processing)
    if state.token_count > 0 && elapsed >= 30 {
        let token_color = if stall > 0.5 {
            lerp_color(theme.secondary, theme.error, stall * 0.3)
        } else {
            theme.secondary
        };
        // Pulse effect on token count
        let token_pulse = ((state.animation_tick as f64 * 0.08).sin() * 0.15 + 0.85).max(0.7);
        let token_color_pulsed =
            lerp_color(token_color, theme.secondary_shimmer, token_pulse * 0.3);
        // 显示 token 用量（紧凑模式）
        let token_display = match &state.token_usage {
            Some(u) if u.prompt_tokens > 0 => {
                format!(" · {} tokens", format_token_count(u.prompt_tokens))
            }
            _ => format!(" · ↓ {}", format_token_count(state.token_count)),
        };
        spans.push(Span::styled(
            token_display,
            Style::default().fg(token_color_pulsed),
        ));
    }

    // Show active tool name if available (with pulse), truncated to keep the line compact
    if let Some(tool_name) = state.current_tool_name.as_ref() {
        let tool_color = if stall > 0.5 {
            lerp_color(theme.info, theme.error, stall * 0.5)
        } else {
            pulse_color(theme.info, state.animation_tick, 4)
        };
        let short: String = tool_name.chars().take(30).collect();
        let label = if tool_name.chars().count() > 30 {
            format!(" · {}...", short)
        } else {
            format!(" · {}", short)
        };
        spans.push(Span::styled(label, Style::default().fg(tool_color)));

        // Show cancel hint when tool is running too long
        if stall > 0.3 {
            let hint_color = lerp_color(theme.secondary, theme.error, stall);
            spans.push(Span::styled(
                " · press Ctrl+C to cancel",
                Style::default().fg(hint_color),
            ));
        }
    }

    vec![
        ratatui::text::Line::from(""),
        ratatui::text::Line::from(spans),
    ]
}

/// TokenWarning thresholds (percentage of context window used)
const TOKEN_WARNING_THRESHOLD: f64 = 80.0; // Show warning
const TOKEN_ERROR_THRESHOLD: f64 = 90.0; // Show error
const AUTO_COMPACT_THRESHOLD: f64 = 92.0; // Auto-compact triggers

/// Render a token usage warning line above the input area.
/// Shows warnings when context window is getting full.
/// Returns 0 or 1 lines.
pub fn token_warning_line(state: &ChatState) -> Vec<ratatui::text::Line<'static>> {
    let theme = state.theme_manager.current();

    // Get current token usage
    let tokens = match &state.token_usage {
        Some(usage) if usage.prompt_tokens > 0 => usage.prompt_tokens,
        Some(usage) if usage.total_tokens > 0 => usage.total_tokens,
        _ => {
            // Fallback: estimate from chat history
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

    let ctx = context_window_tokens(state);
    if ctx == 0 {
        return vec![];
    }

    let pct_used = (tokens as f64 / ctx as f64) * 100.0;
    let pct_remaining = 100.0 - pct_used;

    // Below warning threshold — no warning needed
    if pct_used < TOKEN_WARNING_THRESHOLD {
        return vec![];
    }

    // Check if auto-compact is enabled
    let auto_compact_enabled = std::env::var("STAR_AUTO_COMPACT")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(true); // Default enabled

    let (message, color) = if pct_used >= TOKEN_ERROR_THRESHOLD {
        // Critical: context almost full
        if auto_compact_enabled {
            (
                format!(
                    "Context {:.0}% used · auto-compact will trigger at {:.0}%",
                    pct_used, AUTO_COMPACT_THRESHOLD
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
    } else {
        // Warning: getting close
        if auto_compact_enabled {
            (
                format!(
                    "{:.0}% until auto-compact",
                    AUTO_COMPACT_THRESHOLD - pct_used
                ),
                theme.warning,
            )
        } else {
            (
                format!("Context {:.0}% used · consider running /compact", pct_used),
                theme.warning,
            )
        }
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
        let model = if state.current_model.is_empty() {
            "..."
        } else {
            state.current_model.as_str()
        };
        format!("⊙ {}", model)
    }
}

fn format_token_count(n: u32) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Build the styled status line as a Vec<Line> for full-page rendering.
pub fn build_status_lines(state: &ChatState, width: u16) -> Vec<Line<'static>> {
    let spans: Vec<Span> = build_status_spans(state, width);
    if spans.is_empty() {
        return vec![Line::from("")];
    }
    vec![Line::from(spans)]
}

/// Build the styled spans for the status bar (shared by both renderers).
/// Width-aware: narrow terminals progressively hide less important sections
/// (Claude Code hides token counts / countdowns below 60 columns).
fn build_status_spans(state: &ChatState, width: u16) -> Vec<Span<'static>> {
    let compact = width < 60;
    let minimal = width < 40;
    let mut spans: Vec<Span> = Vec::new();

    // 获取当前主题颜色
    let theme = state.theme_manager.current();

    // ── 1. Model  ⊙ model-name ──────────────────────────────────────────────
    let model = if state.current_model.is_empty() {
        "..."
    } else {
        &state.current_model
    };
    let model_color = theme.success; // 使用主题的 success 颜色
    spans.push(Span::styled(" ⊙ ", Style::default().fg(model_color)));
    spans.push(Span::styled(
        model.to_string(),
        Style::default().fg(model_color),
    ));

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
                let tokens = if usage.prompt_tokens > 0 {
                    usage.prompt_tokens
                } else {
                    usage.total_tokens
                };
                let ctx = context_window_tokens(state);
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

                let token_label = if usage.completion_tokens > 0 {
                    // 显示上下文使用率 + 缓存命中（如果有）
                    let base = format!("{}/{} ({:.1}%)", format_tok(tokens), format_tok(ctx), pct);
                    // 如果有缓存数据，附加缓存命中率
                    if usage.cache_read_tokens > 0 && tokens > 0 {
                        let cache_pct =
                            (usage.cache_read_tokens as f64 / tokens as f64 * 100.0).min(100.0);
                        format!("{} · {:.0}% cached", base, cache_pct)
                    } else {
                        base
                    }
                } else {
                    format!("{}/{} ({:.1}%)", format_tok(tokens), format_tok(ctx), pct)
                };

                let (token_color, bold) = if pct >= 90.0 {
                    (theme.error, true)
                } else if pct >= 75.0 {
                    (theme.error_shimmer, false)
                } else if pct >= 50.0 {
                    (theme.warning_shimmer, false)
                } else {
                    (theme.warning, false)
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
            }
            _ => {
                // ── Fallback: 厂商未返回 usage 时，根据聊天历史估算 ──
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
                    let ctx = context_window_tokens(state);
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
                    spans.push(Span::styled("—", Style::default().fg(dim)));
                }
            }
        }
    }

    // ── 5. Cost + elapsed ────────────────────────────────────────────────────
    // 对标 Claude Code 状态栏的 running cost & duration。成本已由 stream.rs 累计
    // 到 state.total_cost；耗时用会话创建时刻计算。成本为 0（尚未产生 usage）时
    // 只显示耗时。
    if !compact {
        let elapsed = format_elapsed(state.session_started_at.elapsed().as_secs());
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

    // ── 5b. Cache hit rate (only when below 50% — poor utilization) ───────────
    if !compact {
        let total_cache = state.cache_read_tokens + state.cache_creation_tokens;
        if total_cache > 0 {
            let hit_rate = (state.cache_read_tokens as f64 / total_cache as f64 * 100.0) as u32;
            if hit_rate < 50 {
                spans.push(sep());
                spans.push(Span::styled("Cache ", Style::default().fg(theme.inactive)));
                spans.push(Span::styled(
                    format!("{}%", hit_rate),
                    Style::default().fg(theme.inactive),
                ));
            }
        }
    }

    // ── 6. Approval mode (non-default only) ──────────────────────────────────
    match state.approval_mode {
        ApprovalMode::Default => {}
        ApprovalMode::Plan => {
            spans.push(sep());
            spans.push(Span::styled(
                "⏸ plan",
                Style::default().fg(Color::Rgb(0, 102, 102)),
            ));
        }
        ApprovalMode::Yolo => {
            spans.push(sep());
            spans.push(Span::styled(
                "⏵⏵ yolo",
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            ));
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
            crate::ui::vim::VimMode::Command => "-- COMMAND --",
        };
        let vim_color = match state.vim_state.mode {
            crate::ui::vim::VimMode::Normal => theme.success,
            crate::ui::vim::VimMode::Insert => theme.info,
            crate::ui::vim::VimMode::Visual => theme.secondary,
            crate::ui::vim::VimMode::Command => theme.warning,
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
        let avail = crate::core::sandbox::SandboxManager::is_available();
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn shimmer_segments_rejoin_into_the_original_text() {
        // 三段拼回去必须和原文一字不差，否则状态行会漏字或重字
        let text = "Percolating";
        for tick in
            0..(text.width() as u64 + SHIMMER_GAP_COLUMNS as u64 + 4) * SHIMMER_TICKS_PER_COLUMN
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
            if let Some((start, end)) =
                shimmer_columns(text, step as u64 * SHIMMER_TICKS_PER_COLUMN)
            {
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
}
