//! 字符串工具
//!
//! 对标claude-code-main的src/utils/stringUtils.ts
//!
//! # 截断一律走字符边界
//!
//! 这里的 `truncate` / `truncate_with_ellipsis` 原来是字节切片（`&s[..max_len]`）。
//! 纯 ASCII 没问题，但只要字符串里有中文、emoji 或任何多字节字符，切点落在字符
//! 中间就直接 panic —— 而这两个函数的输入恰恰是最可能带中文的东西：工具输出、
//! shell 命令、网页正文、用户提问。
//!
//! 因此 `max_len` 的语义从"字节数"改成"字符数"。纯 ASCII 下两者等价，既有调用点
//! 行为不变；非 ASCII 下从"崩"变成"少留几个字节"。新代码想显式表达意图就用
//! [`truncate_chars`]。

/// 截断到最多 `max_len` 个字符（不会在多字节字符中间切开）
pub fn truncate(s: &str, max_len: usize) -> &str {
    match s.char_indices().nth(max_len) {
        // 第 max_len+1 个字符的起始字节，就是前 max_len 个字符的结束位置
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

/// 截断字符串并添加省略号，返回值总长度不超过 `max_len` 个字符
pub fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        return s.to_string();
    }
    format!("{}...", truncate(s, max_len.saturating_sub(3)))
}

/// 保留开头 `max_chars` 个字符，超长时追加省略号
///
/// 与 [`truncate_with_ellipsis`] 的区别：这里 `max_chars` 只约束正文，省略号是额外
/// 加上去的。适合"保留开头 N 个字符 + 标明被截断"的场景，也正是散落在各处的
/// `format!("{}...", &s[..N])` 想做的事。
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", truncate(s, max_chars))
    }
}

/// 保留末尾 `max_chars` 个字符，超长时在前面加省略号
///
/// 路径类展示常用（"...src/core/tools/web_search.rs"）。
pub fn truncate_start_chars(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let skip = total - max_chars;
    let tail: String = s.chars().skip(skip).collect();
    format!("...{}", tail)
}

/// 首字母大写
pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
    }
}

/// 驼峰命名
pub fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for c in s.chars() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_uppercase().next().unwrap_or(c));
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// 蛇形命名
pub fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let mut prev_uppercase = false;

    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 && !prev_uppercase {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap_or(c));
            prev_uppercase = true;
        } else {
            result.push(c);
            prev_uppercase = false;
        }
    }

    result
}

/// 测试字符串是否为空或空白
pub fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

/// 测试字符串是否不为空或空白
pub fn is_not_blank(s: &str) -> bool {
    !is_blank(s)
}

/// 移除前缀
pub fn remove_prefix<'a>(s: &'a str, prefix: &str) -> &'a str {
    if s.starts_with(prefix) {
        &s[prefix.len()..]
    } else {
        s
    }
}

/// 移除后缀
pub fn remove_suffix<'a>(s: &'a str, suffix: &str) -> &'a str {
    if s.ends_with(suffix) {
        &s[..s.len() - suffix.len()]
    } else {
        s
    }
}

/// 重复字符串
pub fn repeat(s: &str, n: usize) -> String {
    s.repeat(n)
}

/// 填充字符串到指定长度
pub fn pad_left(s: &str, width: usize, fill: char) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        let padding = repeat(&fill.to_string(), width - s.len());
        format!("{}{}", padding, s)
    }
}

/// 填充字符串到指定长度
pub fn pad_right(s: &str, width: usize, fill: char) -> String {
    if s.len() >= width {
        s.to_string()
    } else {
        let padding = repeat(&fill.to_string(), width - s.len());
        format!("{}{}", s, padding)
    }
}

/// 计算字符串相似度（Levenshtein距离）
pub fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    // 用字符数而不是字节数：矩阵维度必须和下面按 char 迭代的次数对齐，否则非 ASCII
    // 输入会去读从未被填过的格子，返回 0（"完全相同"）。
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for i in 0..=len1 {
        matrix[i][0] = i;
    }
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    for (i, c1) in s1.chars().enumerate() {
        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            matrix[i + 1][j + 1] = (matrix[i][j + 1] + 1)
                .min(matrix[i + 1][j] + 1)
                .min(matrix[i][j] + cost);
        }
    }

    matrix[len1][len2]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个 CJK 字符 3 字节：字节切片版本在这里必然 panic。
    const CJK: &str = "上网找的知识点";

    #[test]
    fn truncate_cuts_on_char_boundaries() {
        assert_eq!(truncate(CJK, 3), "上网找");
        // 请求的字符数超过总长时原样返回，而不是越界。
        assert_eq!(truncate(CJK, 999), CJK);
        assert_eq!(truncate(CJK, 0), "");
    }

    #[test]
    fn truncate_matches_old_behavior_for_ascii() {
        assert_eq!(truncate("hello world", 5), "hello");
        assert_eq!(truncate("hi", 5), "hi");
    }

    #[test]
    fn ellipsis_variants_stay_within_budget() {
        // truncate_with_ellipsis: 省略号计入预算
        let out = truncate_with_ellipsis(CJK, 5);
        assert_eq!(out, "上网...");
        assert_eq!(out.chars().count(), 5);
        assert_eq!(truncate_with_ellipsis(CJK, 99), CJK);

        // truncate_chars: 省略号是额外加的
        assert_eq!(truncate_chars(CJK, 2), "上网...");
        assert_eq!(truncate_chars(CJK, 7), CJK);
    }

    #[test]
    fn truncate_start_keeps_the_tail() {
        assert_eq!(truncate_start_chars(CJK, 2), "...识点");
        assert_eq!(truncate_start_chars(CJK, 7), CJK);
        assert_eq!(truncate_start_chars("src/main.rs", 7), "...main.rs");
    }

    #[test]
    fn emoji_are_not_split_either() {
        // 4 字节字符，落在任何非 4 的倍数上都会 panic。
        let s = "🌟🌟🌟";
        assert_eq!(truncate(s, 1), "🌟");
        assert_eq!(truncate_chars(s, 2), "🌟🌟...");
    }

    #[test]
    fn levenshtein_counts_characters_not_bytes() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        // 每字符 3 字节；按字节建矩阵时这里会返回 0。
        assert_eq!(levenshtein_distance("知识点", "知识"), 1);
        assert_eq!(levenshtein_distance("知识点", "知识点"), 0);
    }
}
