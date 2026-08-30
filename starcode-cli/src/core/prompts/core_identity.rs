use crate::core::prompts::loader;

pub fn render(is_thinking_model: bool) -> String {
    let reasoning_section = if is_thinking_model {
        "- **Reasoning**: Use native reasoning internally; do NOT emit `<thinking>`, `<think>`, `<plan>`, or any XML tags unless the user explicitly asks."
    } else {
        "- **Reasoning**: Think before acting; keep reasoning internal. Do NOT emit `<thinking>`, `<think>`, `<plan>`, or any XML reasoning tags in your response."
    };

    // Load core identity from file and replace placeholder
    let template = loader::load_prompt("system-prompt-core-identity.md");
    loader::render_template(&template, &[("reasoning_section", reasoning_section)])
}
