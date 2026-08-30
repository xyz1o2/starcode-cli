use crate::core::prompts::loader;

pub fn render(_is_thinking_model: bool) -> String {
    // Load agent mode from file (external dir overrides embedded)
    loader::load_prompt("system-prompt-agent-mode.md")
}
