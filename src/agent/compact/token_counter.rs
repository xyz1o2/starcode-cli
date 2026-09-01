use crate::types::StarMessage;

/// 估算文本的 token 数量（约 4 个字符 = 1 个 token）
pub fn estimate_text_tokens(text: &str) -> usize {
    // 对于中文等多字节字符，每个字符约 1-2 个 token
    // 对于英文等单字节字符，约 4 个字符 = 1 个 token
    // 这里使用简单的估算：字符数 / 3（保守估计）
    let char_count = text.chars().count();
    let byte_count = text.len();

    // 如果主要是 ASCII 字符，使用字符数 / 4
    // 如果主要是多字节字符，使用字符数 / 2
    let ascii_ratio = byte_count as f64 / char_count.max(1) as f64;

    if ascii_ratio > 1.5 {
        // 主要是 ASCII 字符
        (char_count + 3) / 4
    } else {
        // 主要是多字节字符（如中文）
        (char_count + 1) / 2
    }
}

/// 计算单个消息的 token 数量
pub fn count_message_tokens(msg: &StarMessage) -> usize {
    let mut tokens = 0;

    // 角色 token（约 4 个 token）
    tokens += 4;

    // 内容 token
    if let Some(content) = &msg.content {
        tokens += estimate_text_tokens(content);
    }

    // 推理内容 token
    if let Some(reasoning) = &msg.reasoning_content {
        tokens += estimate_text_tokens(reasoning);
    }

    // 工具调用 token
    if let Some(tool_calls) = &msg.tool_calls {
        for tc in tool_calls {
            // 工具调用元数据（约 20 个 token）
            tokens += 20;
            // 工具名称
            tokens += estimate_text_tokens(&tc.function.name);
            // 工具参数
            tokens += estimate_text_tokens(&tc.function.arguments);
        }
    }

    // 工具调用 ID（约 10 个 token）
    if msg.tool_call_id.is_some() {
        tokens += 10;
    }

    tokens
}

/// 计算消息列表的总 token 数量
pub fn count_tokens(messages: &[StarMessage]) -> usize {
    messages.iter().map(|msg| count_message_tokens(msg)).sum()
}

/// 计算消息列表中指定范围的 token 数量
pub fn count_tokens_range(messages: &[StarMessage], start: usize, end: usize) -> usize {
    messages[start..end]
        .iter()
        .map(|msg| count_message_tokens(msg))
        .sum()
}

/// 查找第一个超过指定 token 数量的消息索引
pub fn find_first_exceeding_token_limit(messages: &[StarMessage], limit: usize) -> Option<usize> {
    let mut total = 0;
    for (i, msg) in messages.iter().enumerate() {
        total += count_message_tokens(msg);
        if total > limit {
            return Some(i);
        }
    }
    None
}

/// 计算压缩后的大致 token 数量
pub fn estimate_compressed_tokens(original_tokens: usize, compression_ratio: f64) -> usize {
    (original_tokens as f64 * compression_ratio) as usize
}
