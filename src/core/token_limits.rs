pub const DEFAULT_MAX_TOKENS: usize = 0;

/// 简单的 Token 估算：按 4 字符/token 计算
/// 这是业界常用的粗略估算方法 (Rule of thumb: 1 token ~= 4 chars in English)
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}
