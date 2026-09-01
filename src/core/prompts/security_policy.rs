use crate::core::prompts::loader;

pub fn render() -> String {
    // Load security policy from file (external dir overrides embedded)
    loader::load_prompt("system-prompt-security-policy.md")
}

pub fn bash_injection_detection_prompt() -> String {
    crate::core::policy::security_prompts::bash_injection_detection_prompt()
}
