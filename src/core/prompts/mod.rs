pub mod agent_mode;
pub mod core_identity;
pub mod env_info;
pub mod key_scenarios;
pub mod loader;
pub mod main_system;
pub mod prompt_cache;
pub mod reminders;
pub mod security_policy;
pub mod skills;
pub mod summary;
pub mod system_prompt_type;
pub mod task_agent_usage;
pub mod tool_catalog;
pub mod tool_descriptions;
pub mod tool_list;

#[cfg(test)]
mod tests;
 

use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "src/core/prompts/system-prompts"]
pub struct SystemPrompts;
