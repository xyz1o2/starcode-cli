use crate::core::prompts::loader;

pub fn render() -> String {
    // Load task agent usage from file (external dir overrides embedded)
    loader::load_prompt("system-prompt-task-agent-usage.md")
}
