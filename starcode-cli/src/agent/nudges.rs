/// Recovery nudge messages injected into the conversation when the model
/// produces unusable or incomplete responses.
///
/// These are NOT system prompts — they are injected as system-role messages
/// during the agent loop to nudge the model back on track.
///
/// All messages MUST be in English (the model's primary language).  Never
/// use Chinese or any other language here — the model is prompted in English
/// and mixed-language nudges degrade output quality.

// ── Empty / thinking-only response recovery ──

/// Injected when the model produces a truly empty response
/// (no content, no reasoning, no tool calls).
pub const NUDGE_EMPTY_RESPONSE: &str =
    "Your previous response was empty. You MUST respond to the user's request. \
     Use the available tools to take concrete action, or at minimum explain \
     what you need to proceed.";

/// Injected when the model produced reasoning but no content/tool calls
/// (common with DeepSeek reasoning models that think but never respond).
pub const NUDGE_THINKING_ONLY: &str =
    "You have completed your thinking. Now you MUST produce a response: \
     either use a tool to take concrete action, or provide a text answer \
     to the user's request. Do not think further — respond now.";

// ── Output truncation recovery ──

/// Injected when the model hit its output token limit (finish_reason="length")
/// and the response was truncated mid-stream.
pub const NUDGE_TRUNCATED_STREAM: &str =
    "Your previous response was cut off because you exceeded the output token limit. \
     Continue exactly where you left off — do not repeat what you already said. \
     Be concise and complete your work.";

// ── Guard rails ──

/// Injected when the model returns a short text-only response
/// (less than 20 chars) and is about to finish — asks for a proper conclusion.
pub const NUDGE_CONCLUSION_REQUEST: &str =
    "[CONCLUSION_REQUEST] You are about to finish but have not provided a conclusion. \
     Provide a brief summary of what was done, any changes made, and the current status. \
     Be concise but complete.";

/// Injected when the model returns a text-only response to what looks like
/// a coding task — pushes the model to use tools.
pub const NUDGE_ACTION_REQUIRED: &str =
    "You responded with a text explanation but did not use any tools. \
     The user request requires concrete action — use the available \
     tools (Grep, Read, Edit, etc.) to make progress. \
     Do at least one tool call before responding.";
