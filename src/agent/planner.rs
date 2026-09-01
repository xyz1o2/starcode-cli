use crate::llm::client::StarClient;
use crate::types::StarMessage;

/// Planner system prompt - loaded from centralized .md file
/// (external dir overrides embedded, cached via loader).

pub struct Planner;

impl Planner {
    pub async fn make_plan(
        client: &StarClient,
        user_input: &str,
        model: Option<String>,
        tool_catalog: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let planner_prompt = crate::core::prompts::loader::load_prompt("planner-system.md");
        let system = StarMessage::system(
            crate::core::prompts::loader::render_template(
                &planner_prompt,
                &[("tool_catalog", tool_catalog)],
            )
        );

        let user = StarMessage::user(user_input.to_string());

        let resp = client.chat(vec![system, user], None, model, None).await?;
        let msg = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default();
        Ok(msg)
    }
}
