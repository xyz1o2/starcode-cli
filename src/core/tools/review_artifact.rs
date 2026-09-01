use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct ReviewArtifactTool;

impl ReviewArtifactTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ReviewArtifactParams {
    pub artifact: String,
    pub title: Option<String>,
    pub annotations: Vec<Annotation>,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Annotation {
    pub line: Option<u32>,
    pub message: String,
    pub severity: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ReviewArtifactOutput {
    pub artifact: String,
    pub title: Option<String>,
    pub annotation_count: usize,
    pub summary: Option<String>,
}

pub struct ReviewArtifactInvocation {
    params: ReviewArtifactParams,
}

impl ToolInvocation for ReviewArtifactInvocation {
    fn get_description(&self) -> String {
        format!(
            "Review artifact: {}",
            self.params.title.as_deref().unwrap_or("untitled")
        )
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
            let artifact = params.artifact.clone();
            let title = params
                .title
                .unwrap_or_else(|| "Untitled Review".to_string());
            let annotation_count = params.annotations.len();
            let has_summary = params.summary.is_some();
            let summary = params
                .summary
                .unwrap_or_else(|| "Review completed".to_string());

            // In a real implementation, this would:
            // 1. Process the artifact and annotations
            // 2. Generate a review report
            // 3. Return the review results

            // For now, return a placeholder response
            Ok(ToolResult {
                llm_content: Some(format!(
                    "Reviewed '{}' with {} annotations",
                    title, annotation_count
                )),
                return_display: Some(format!(
                    "Review completed: {} annotations",
                    annotation_count
                )),
                output: serde_json::to_string(&ReviewArtifactOutput {
                    artifact: if artifact.len() > 100 {
                        format!("{}...", &artifact[..97])
                    } else {
                        artifact
                    },
                    title: Some(title),
                    annotation_count,
                    summary: Some(summary),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "annotation_count": annotation_count,
                    "has_summary": has_summary
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for ReviewArtifactTool {
    fn name(&self) -> &str {
        "review_artifact"
    }

    fn display_name(&self) -> &str {
        "ReviewArtifact"
    }

    fn description(&self) -> &str {
        "对代码片段、文档等内容进行审查，提供行内注释和反馈。(Review code snippets, documents, etc., providing inline annotations and feedback.)"
    }

    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "artifact": {
                    "type": "string",
                    "description": "要审查的内容（代码、文档等）(Content to review - code, documentation, etc.)"
                },
                "title": {
                    "type": "string",
                    "description": "审查标题 (Review title)"
                },
                "annotations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "line": {
                                "type": "integer",
                                "description": "行号（可选）(Line number, optional)"
                            },
                            "message": {
                                "type": "string",
                                "description": "注释内容 (Annotation message)"
                            },
                            "severity": {
                                "type": "string",
                                "enum": ["info", "warning", "error", "suggestion"],
                                "description": "严重程度，默认info (Severity level, defaults to info)"
                            }
                        },
                        "required": ["message"]
                    },
                    "description": "审查注释列表 (List of review annotations)"
                },
                "summary": {
                    "type": "string",
                    "description": "审查总结 (Review summary)"
                }
            },
            "required": ["artifact", "annotations"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ReviewArtifactParams = serde_json::from_value(params)?;
        Ok(Box::new(ReviewArtifactInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
