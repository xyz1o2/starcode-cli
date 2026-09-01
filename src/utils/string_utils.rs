/// 字符串工具
///
/// 对标claude-code-main的src/utils/stringUtils.ts

/// 截断字符串
pub fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

/// 截断字符串并添加省略号
pub fn truncate_with_ellipsis(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
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
    let len1 = s1.len();
    let len2 = s2.len();

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
