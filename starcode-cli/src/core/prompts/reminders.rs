use crate::core::prompts::loader;

pub fn render(is_thinking_model: bool) -> String {
    let reasoning_line = if is_thinking_model {
        "1. Think internally; then call tools—never narrate intent as output."
    } else {
        "1. Think briefly; call tools immediately—do not explain what you are about to do."
    };

    // Load reminders from file and replace placeholder
    let template = loader::load_prompt("system-prompt-reminders.md");
    loader::render_template(&template, &[("reasoning_line", reasoning_line)])
}
