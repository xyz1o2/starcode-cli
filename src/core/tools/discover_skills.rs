use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct DiscoverSkillsTool;

impl DiscoverSkillsTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct DiscoverSkillsParams {
    pub description: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DiscoverSkillsOutput {
    pub results: Vec<SkillResult>,
    pub count: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct SkillResult {
    pub name: String,
    pub description: String,
    pub score: f64,
}

pub struct DiscoverSkillsInvocation {
    params: DiscoverSkillsParams,
}

impl ToolInvocation for DiscoverSkillsInvocation {
    fn get_description(&self) -> String {
        format!("Discover skills: {}", self.params.description)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        _update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let params = self.params.clone();
        Box::pin(async move {
            let description = params.description.clone();
            let limit = params.limit.unwrap_or(5);

            // In a real implementation, this would:
            // 1. Get skill index
            // 2. Search skills using TF-IDF
            // 3. Return matching skills

            // For now, return a placeholder response
            let results = vec![
                SkillResult {
                    name: "git_commit".to_string(),
                    description: "Create git commits with AI assistance".to_string(),
                    score: 0.95,
                },
                SkillResult {
                    name: "code_review".to_string(),
                    description: "Review code for bugs and improvements".to_string(),
                    score: 0.88,
                },
                SkillResult {
                    name: "refactor".to_string(),
                    description: "Refactor code for better structure".to_string(),
                    score: 0.82,
                },
            ];

            let results = results.into_iter().take(limit).collect::<Vec<_>>();
            let count = results.len();

            Ok(ToolResult {
                llm_content: Some(format!("Found {} skills matching '{}'", count, description)),
                return_display: Some(format!("{} skills found", count)),
                output: serde_json::to_string(&DiscoverSkillsOutput {
                    results,
                    count,
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "count": count,
                    "description": description
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for DiscoverSkillsTool {
    fn name(&self) -> &str {
        "discover_skills"
    }

    fn display_name(&self) -> &str {
        "DiscoverSkills"
    }

    fn description(&self) -> &str {
        "发现可用的技能。用于查找和加载特定任务的技能。(Discover available skills. Used to find and load skills for specific tasks.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "要做什么的描述。请具体说明，例如 \"deploy a Next.js app to Cloudflare Workers\" 而不是 \"deploy\"。"
                },
                "limit": {
                    "type": "integer",
                    "description": "最大返回结果数，默认5 (Maximum number of results to return, default: 5)"
                }
            },
            "required": ["description"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: DiscoverSkillsParams = serde_json::from_value(params)?;
        Ok(Box::new(DiscoverSkillsInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}