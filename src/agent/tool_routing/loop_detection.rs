use std::collections::VecDeque;
use std::sync::OnceLock;

/// 解析工具循环重复阈值
pub(crate) fn resolved_tool_loop_repeat_threshold() -> usize {
    static LOOP_REPEAT_THRESHOLD: OnceLock<usize> = OnceLock::new();
    *LOOP_REPEAT_THRESHOLD.get_or_init(|| {
        std::env::var("STAR_TOOL_LOOP_REPEAT_THRESHOLD")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(2, 10)
    })
}

/// 检测工具循环
pub(crate) fn detect_tool_loop(
    history: &VecDeque<String>,
    repeat_threshold: usize,
) -> Option<String> {
    let latest = history.back()?;

    // 使用每工具阈值：bash/search 工具预期会合法重复
    let effective_threshold = adjusted_threshold_for_signature(latest, repeat_threshold);

    let mut repeated = 0usize;
    for sig in history.iter().rev() {
        if sig == latest {
            repeated += 1;
        } else {
            break;
        }
    }
    if repeated >= effective_threshold {
        return Some(format!(
            "same tool pattern repeated {} times (`{}`; threshold={})",
            repeated,
            truncate_chars(latest, 120),
            effective_threshold,
        ));
    }

    if history.len() >= 4 {
        let n = history.len();
        let a = &history[n - 4];
        let b = &history[n - 3];
        let c = &history[n - 2];
        let d = &history[n - 1];
        if a == c && b == d && a != b {
            return Some(format!(
                "alternating tool pattern detected (`{}` <-> `{}`)",
                truncate_chars(a, 90),
                truncate_chars(b, 90)
            ));
        }
    }

    None
}

/// 为签名调整阈值
fn adjusted_threshold_for_signature(signature: &str, base_threshold: usize) -> usize {
    // 对于 bash/search 工具，允许更高的重复次数
    if signature.starts_with("bash(")
        || signature.starts_with("search(")
        || signature.starts_with("rg(")
    {
        base_threshold + 2
    } else {
        base_threshold
    }
}

/// 截断字符串
fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.len() <= max_chars {
        input.to_string()
    } else {
        format!("{}...", &input[..max_chars])
    }
}
