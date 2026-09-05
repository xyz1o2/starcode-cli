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
///
/// 转发到共享实现。原来这里是 `&input[..max_chars]` —— 字节切片，而签名里几乎必然
/// 带用户输入（搜索词、文件路径）。也就是说循环检测一旦真的命中，格式化告警信息时
/// 自己就先 panic 了，正好是最不该崩的时刻。
fn truncate_chars(input: &str, max_chars: usize) -> String {
    crate::utils::string_utils::truncate_chars(input, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(sigs: &[&str]) -> VecDeque<String> {
        sigs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn repeated_signatures_are_detected() {
        let h = history(&["Read(a)", "Read(a)", "Read(a)", "Read(a)"]);
        let hit = detect_tool_loop(&h, 4).expect("4 identical calls should trip the detector");
        assert!(hit.contains("repeated 4 times"));
    }

    #[test]
    fn alternating_signatures_are_detected() {
        let h = history(&["Read(a)", "Edit(b)", "Read(a)", "Edit(b)"]);
        let hit = detect_tool_loop(&h, 9).expect("A-B-A-B should trip the detector");
        assert!(hit.contains("alternating"));
    }

    /// 命中时要格式化签名 —— 签名里带中文时旧实现在这一步 panic。
    #[test]
    fn a_cjk_signature_does_not_panic_when_reported() {
        let long = format!("WebSearch(query=\"{}\")", "上网找的知识点".repeat(40));
        let h = history(&[&long, &long, &long, &long]);
        let hit = detect_tool_loop(&h, 4).expect("should still be detected");
        assert!(hit.contains("repeated 4 times"));
        assert!(hit.contains("上网找的知识点"));
    }

    #[test]
    fn distinct_signatures_are_not_a_loop() {
        // 每次换关键词的联网搜索：签名各不相同，这个检测器看不到 —— 所以
        // "不停地找" 不能指望它兜住，得从源头（工具结果不再被毁）解决。
        let h = history(&[
            "WebSearch(q=a)",
            "WebSearch(q=b)",
            "WebSearch(q=c)",
            "WebSearch(q=d)",
        ]);
        assert!(detect_tool_loop(&h, 4).is_none());
    }
}
