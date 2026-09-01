use crate::core::prompts::loader;

pub fn render(_is_thinking_model: bool) -> String {
    // Load system prompt from file (external dir overrides embedded)
    loader::load_prompt("system-prompt.md")
}
