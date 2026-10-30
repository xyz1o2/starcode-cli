//! 转圈动画的像素层：spinner 帧、动词表、shimmer、stall 与 warmup 强度、颜色脉冲。
//!
//! 这些东西只回答这一帧画什么颜色、什么字，不碰版面布局。

use super::*;

/// spinner 字形：会转的十帧 braille。
///
/// 此前是 `●`/`○` 正反交替。两态、整圈 8 帧 ≈ 300ms，在终端里读起来是「抖」
/// 而不是「转」；更糟的是 `○` 那个空心圈本身传达的是"空闲"，而它恰恰是
/// "正在干活"时最常停住的那一帧。
///
/// 换 braille 还有一层安全性：`●`/`○` 是 East Asian **Ambiguous** 宽度类，
/// 当初要小心翼翼正是怕 CJK 终端里变成双宽、与单宽字符交替时把整行文字推来推去
/// （Alternating wide/narrow glyphs move subsequent text）。braille pattern
/// (U+2800..U+28FF) 是 **Neutral**，任何终端下都是单宽，比被替换掉的两个字符
/// 更不容易出问题。
pub(super) const BRAILLE_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// spinner 动词表。
///
/// 这一版直接采用 Claude Code 真实的 187 条（随 source map 泄露的 `getSpinnerVerbs`
/// 全量），而不是此前那份「数量对齐、内容自造」的 200 条：程序化比对下来两者只有
/// 49 条重合。那份表里有 16 个 Napping/Snoozing/Loafing 类的闲置动词（真实表只有
/// Dilly-dallying 一个）、13 个 Reading/Compiling/Indexing 类的字面任务词（真实表刻意
/// 不用——那是在编造一个并不存在的具体动作），外加 14 条重复项把 `hash % len` 的分布
/// 打歪。
///
/// 换回真实表有两个实际收益：动词池的中心是「煞有介事地干活」而不是「闲着」，
/// 一轮四分钟的长等待配 `Napping` 读起来像玩笑；以及 `Thinking` 在场——那是唯一
/// 在任何时刻都不会说错的词。
pub(super) const SPINNER_VERBS: &[&str] = &[
    "Accomplishing",
    "Actioning",
    "Actualizing",
    "Architecting",
    "Baking",
    "Beaming",
    "Beboppin'",
    "Befuddling",
    "Billowing",
    "Blanching",
    "Bloviating",
    "Boogieing",
    "Boondoggling",
    "Booping",
    "Bootstrapping",
    "Brewing",
    "Bunning",
    "Burrowing",
    "Calculating",
    "Canoodling",
    "Caramelizing",
    "Cascading",
    "Catapulting",
    "Cerebrating",
    "Channeling",
    "Channelling",
    "Choreographing",
    "Churning",
    "Clauding",
    "Coalescing",
    "Cogitating",
    "Combobulating",
    "Composing",
    "Computing",
    "Concocting",
    "Considering",
    "Contemplating",
    "Cooking",
    "Crafting",
    "Creating",
    "Crunching",
    "Crystallizing",
    "Cultivating",
    "Deciphering",
    "Deliberating",
    "Determining",
    "Dilly-dallying",
    "Discombobulating",
    "Doing",
    "Doodling",
    "Drizzling",
    "Ebbing",
    "Effecting",
    "Elucidating",
    "Embellishing",
    "Enchanting",
    "Envisioning",
    "Evaporating",
    "Fermenting",
    "Fiddle-faddling",
    "Finagling",
    "Flambéing",
    "Flibbertigibbeting",
    "Flowing",
    "Flummoxing",
    "Fluttering",
    "Forging",
    "Forming",
    "Frolicking",
    "Frosting",
    "Gallivanting",
    "Galloping",
    "Garnishing",
    "Generating",
    "Gesticulating",
    "Germinating",
    "Gitifying",
    "Grooving",
    "Gusting",
    "Harmonizing",
    "Hashing",
    "Hatching",
    "Herding",
    "Honking",
    "Hullaballooing",
    "Hyperspacing",
    "Ideating",
    "Imagining",
    "Improvising",
    "Incubating",
    "Inferring",
    "Infusing",
    "Ionizing",
    "Jitterbugging",
    "Julienning",
    "Kneading",
    "Leavening",
    "Levitating",
    "Lollygagging",
    "Manifesting",
    "Marinating",
    "Meandering",
    "Metamorphosing",
    "Misting",
    "Moonwalking",
    "Moseying",
    "Mulling",
    "Mustering",
    "Musing",
    "Nebulizing",
    "Nesting",
    "Newspapering",
    "Noodling",
    "Nucleating",
    "Orbiting",
    "Orchestrating",
    "Osmosing",
    "Perambulating",
    "Percolating",
    "Perusing",
    "Philosophising",
    "Photosynthesizing",
    "Pollinating",
    "Pondering",
    "Pontificating",
    "Pouncing",
    "Precipitating",
    "Prestidigitating",
    "Processing",
    "Proofing",
    "Propagating",
    "Puttering",
    "Puzzling",
    "Quantumizing",
    "Razzle-dazzling",
    "Razzmatazzing",
    "Recombobulating",
    "Reticulating",
    "Roosting",
    "Ruminating",
    "Sautéing",
    "Scampering",
    "Schlepping",
    "Scurrying",
    "Seasoning",
    "Shenaniganing",
    "Shimmying",
    "Simmering",
    "Skedaddling",
    "Sketching",
    "Slithering",
    "Smooshing",
    "Sock-hopping",
    "Spelunking",
    "Spinning",
    "Sprouting",
    "Stewing",
    "Sublimating",
    "Swirling",
    "Swooping",
    "Symbioting",
    "Synthesizing",
    "Tempering",
    "Thinking",
    "Thundering",
    "Tinkering",
    "Tomfoolering",
    "Topsy-turvying",
    "Transfiguring",
    "Transmuting",
    "Twisting",
    "Undulating",
    "Unfurling",
    "Unravelling",
    "Vibing",
    "Waddling",
    "Wandering",
    "Warping",
    "Whatchamacalliting",
    "Whirlpooling",
    "Whirring",
    "Whisking",
    "Wibbling",
    "Working",
    "Wrangling",
    "Zesting",
    "Zigzagging",
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

/// 静默多久之后开始"变暖"（秒）。
///
/// 对标 Claude Code v2.1.138 的 changelog："the spinner now warms to amber after
/// 10 seconds to signal Claude is still working"。语义是**安慰**——告诉用户模型
/// 还在算，不是在报警。
///
/// 此前这里是 3 秒、且直接插值到错误红。5 秒就变红的后果是它恒定触发：思考段落
/// 之间静默几十秒是常态，首 token 等一分钟更常见，于是整轮几乎全程红字加一句
/// "press Ctrl+C to cancel"。红色一旦成了默认状态就不再携带任何信息 —— 用户既
/// 分不清"在跑"和"卡死"，也会在真正卡死时不再当回事。
pub(super) const WARMUP_THRESHOLD_SECS: u64 = 10;
/// 变暖到满格所需时间（秒）。10s 起、25s 满，之后一直保持。
pub(super) const WARMUP_TRANSITION_SECS: u64 = 15;

/// 工具执行 stall 阈值（秒）。
///
/// 工具确有硬上限（`control_requests.rs` 的 240/120/180 分档），所以工具侧可以
/// 保留真正的报警：超过 60 秒仍未结束才算异常。
pub(super) const TOOL_STALL_WARN_SECS: u64 = 30;
pub(super) const TOOL_STALL_FULL_SECS: u64 = 60;

/// 当前正在执行的工具迟迟不结束时，报警强度 0.0–1.0。无工具时恒为 0。
pub(super) fn tool_stall_intensity(
    tool_started_at: Option<std::time::Instant>,
    animation_tick: u64,
) -> f64 {
    let Some(started) = tool_started_at else {
        return 0.0;
    };
    let elapsed = started.elapsed().as_secs();
    if elapsed < TOOL_STALL_WARN_SECS {
        return 0.0;
    }
    let span = (TOOL_STALL_FULL_SECS - TOOL_STALL_WARN_SECS).max(1) as f64;
    let progress = ((elapsed - TOOL_STALL_WARN_SECS) as f64 / span).min(1.0);
    let pulse = (animation_tick as f64 * 0.15).sin() * 0.1;
    (progress + pulse).clamp(0.0, 1.0)
}

/// 等待模型出下一个 token 时的"变暖"强度 0.0–1.0。
///
/// 只表示"还在等"，不表示"出问题"。这是本轮修复的核心：把长等待从报警通道
/// 挪进安抚通道，红色重新留给真正的异常。
pub(super) fn warmup_intensity(
    last_token_time: Option<std::time::Instant>,
    animation_tick: u64,
) -> f64 {
    let Some(last) = last_token_time else {
        return 0.0;
    };
    let silent = last.elapsed().as_secs();
    if silent < WARMUP_THRESHOLD_SECS {
        return 0.0;
    }
    let progress =
        ((silent - WARMUP_THRESHOLD_SECS) as f64 / WARMUP_TRANSITION_SECS as f64).min(1.0);
    let pulse = (animation_tick as f64 * 0.1).sin() * 0.08;
    (progress + pulse).clamp(0.0, 1.0)
}

/// Interpolate between two colors based on factor (0.0 = a, 1.0 = b)
pub(super) fn lerp_color(a: Color, b: Color, factor: f64) -> Color {
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

/// 呼吸脉动：在 0.7–1.0 之间明暗循环，给静息状态一点生命感。
pub(super) fn pulse_color(base: Color, tick: u64, speed: u64) -> Color {
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

/// Build a progress bar string for long operations (shows elapsed / expected)
pub(super) fn progress_bar_str(tick: u64, width: usize) -> String {
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

/// 返回仍在执行的工具中最后开始的时间，作为工具状态的权威来源。
pub(super) fn latest_active_tool_started_at(state: &ChatState) -> Option<std::time::Instant> {
    state.tool_started_at.values().max().copied()
}

/// spinner 字形的颜色。
///
/// 三段语义，和 Claude Code 对齐：
/// - 正常：主题色 + 轻微脉动
/// - 变暖（静默 ≥10s，无工具在跑）：琥珀色，含义是"还在算"
/// - 报警（有工具超 60s 未结束）：错误红，含义才是"可能卡住"
///
/// 顺序很重要：工具报警压过变暖。反过来会让"正在跑一个 4 分钟的构建"显示成
/// 琥珀安抚色，而那个构建恰恰是最该让人知道它在占着资源的情况。
pub(super) fn spinner_color(
    base_color: Color,
    warmup: f64,
    stall: f64,
    animation_tick: u64,
) -> Color {
    const WARM_COLOR: Color = Color::Rgb(222, 160, 40);
    const STALLED_COLOR: Color = Color::Rgb(171, 43, 63); // ERROR_RED from Claude Code

    // 顺序即优先级：工具报警压过变暖。反过来会让"正在跑一个 4 分钟的构建"显示成
    // 琥珀安抚色，而那个构建恰恰是最该让人知道它在占着资源的情况。
    if stall > 0.0 {
        lerp_color(base_color, STALLED_COLOR, stall)
    } else if warmup > 0.0 {
        lerp_color(base_color, WARM_COLOR, warmup)
    } else {
        pulse_color(base_color, animation_tick, 3)
    }
}

/// 文字类 span 的颜色：只有工具真报警时才偏红，变暖不参与。
///
/// 变暖只染 spinner 字形 —— 整行文字一起变琥珀会和确认卡、错误提示撞色，
/// 反而削弱了 spinner 本身的信号强度。
pub(super) fn text_color(normal: Color, stall: f64) -> Color {
    if stall > 0.0 {
        lerp_color(normal, Color::Rgb(171, 43, 63), stall * 0.7)
    } else {
        normal
    }
}

/// 流光光带的半宽（列）：光带总宽 `2 * HALF + 1`，对标 Claude Code 的
/// `glimmerIndex ± 1`（GlimmerMessage.tsx）。
pub(super) const SHIMMER_BAND_HALF_WIDTH: i64 = 1;

/// 两次扫光之间的间隔（列）：周期取 `文本宽度 + GAP`，光带走到文本之外的
/// 那段时间就是间隔。对标 Claude Code 的 `cycleLength = width + 20`。
pub(super) const SHIMMER_GAP_COLUMNS: i64 = 20;

/// 光带每前进一列要经过的动画帧数。
///
/// `animation_tick` 每绘制一帧 +1，帧间隔 33–50ms（见 runtime.rs 的
/// target_framerate），4 帧 ≈ 165–200ms，对上 Claude Code 的 200ms
/// （`glimmerSpeed`，50ms 时钟下每 4 帧走一列）。
pub(super) const SHIMMER_TICKS_PER_COLUMN: u64 = 4;

/// Render a shimmer effect on a text string.
/// Returns a vector of (text, is_shimmer) segments.
///
/// 对标 Claude Code 的 GlimmerMessage：高亮是一条 3 列宽的窄光带，从右向左
/// 扫过文字，扫出去之后停一会儿再从右边进来。
///
/// 按**显示列**切分而不是按字符数：CJK 字符占 2 列，按字符数算光带在中文里
/// 会走得比看起来快一倍；更要紧的是，之前这里的光带宽 10 个字符，遇到
/// 「正在完善前端页面」这种 8 字短语时整条几乎永远被点亮 —— 看上去就是不动。
pub(super) fn shimmer_segments(text: &str, animation_tick: u64) -> Vec<(&str, bool)> {
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
pub(super) const MAX_TODO_VERB_WIDTH: usize = 32;

/// 本轮对话固定使用的随机动词。
///
/// 对标 Claude Code：动词在 spinner 挂载时抽一次就定下来
/// （`const [randomVerb] = useState(() => sample(getSpinnerVerbs()))`），整轮
/// 不再变 —— 那一行的动感来自文字上扫过的流光，不是来自换词。之前这里写的是
/// `SPINNER_VERBS[elapsed / 5 % len]`，每 5 秒跳一个词，反而不像参照实现。
///
/// 这里没有"挂载"可挂，就拿每轮递增的 `active_message_id` 当种子：轮内稳定，
/// 换轮换词。哈希一遍是为了让相邻两轮不至于按 SPINNER_VERBS 的顺序往下念。
pub(super) fn turn_spinner_verb(state: &ChatState) -> &'static str {
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
pub(super) fn is_latin_phrase(text: &str) -> bool {
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
pub(super) fn in_progress_todo_verb(state: &ChatState) -> Option<String> {
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
