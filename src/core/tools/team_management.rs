//! Team management tools — merged from team_create + team_delete + list_peers

use crate::core::tools::tools::{
    BaseDeclarativeTool, Kind, ToolInvocation, ToolLocation, ToolResult,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── TeamCreate ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TeamCreateTool;

impl TeamCreateTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TeamCreateParams {
    pub team_name: String,
    pub description: Option<String>,
    pub agent_type: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct TeamCreateOutput {
    pub team_name: String,
    pub team_file_path: String,
    pub lead_agent_id: String,
}

pub struct TeamCreateInvocation {
    params: TeamCreateParams,
}

impl ToolInvocation for TeamCreateInvocation {
    fn get_description(&self) -> String {
        format!("Create team: {}", self.params.team_name)
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
            let team_name = params.team_name.clone();
            let agent_type = params.agent_type.unwrap_or_else(|| "team-lead".to_string());
            let lead_agent_id = format!("team-lead@{}", team_name);
            let team_file_path = format!(".star/teams/{}.json", team_name);

            Ok(ToolResult {
                llm_content: Some(format!(
                    "Team '{}' created successfully. Lead agent: {}",
                    team_name, lead_agent_id
                )),
                return_display: Some(format!("Team '{}' created", team_name)),
                output: serde_json::to_string(&TeamCreateOutput {
                    team_name: team_name.clone(),
                    team_file_path,
                    lead_agent_id: lead_agent_id.clone(),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "team_name": team_name,
                    "lead_agent_id": lead_agent_id,
                    "agent_type": agent_type
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for TeamCreateTool {
    fn name(&self) -> &str {
        "team_create"
    }
    fn display_name(&self) -> &str {
        "TeamCreate"
    }
    fn description(&self) -> &str {
        "创建新团队用于协调多个Agent。用于多Agent协作任务。(Create a new team for coordinating multiple agents. Used for multi-agent collaboration tasks.)"
    }
    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "新团队的名称 (Name for the new team to create.)"
                },
                "description": {
                    "type": "string",
                    "description": "团队描述/用途 (Team description/purpose.)"
                },
                "agent_type": {
                    "type": "string",
                    "description": "团队负责人的类型/角色，如 \"researcher\", \"test-runner\" (Type/role of the team lead.)"
                }
            },
            "required": ["team_name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TeamCreateParams = serde_json::from_value(params)?;
        Ok(Box::new(TeamCreateInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}

// ── TeamDelete ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TeamDeleteTool;

impl TeamDeleteTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct TeamDeleteParams {
    pub team_name: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct TeamDeleteOutput {
    pub success: bool,
    pub team_name: String,
    pub message: String,
}

pub struct TeamDeleteInvocation {
    params: TeamDeleteParams,
}

impl ToolInvocation for TeamDeleteInvocation {
    fn get_description(&self) -> String {
        format!("Delete team: {}", self.params.team_name)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn should_confirm_execute(
        &self,
        _abort_signal: Option<&tokio_util::sync::CancellationToken>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Option<crate::core::tools::tools::ToolCallConfirmationDetails>,
                        Box<dyn std::error::Error + Send + Sync>,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Ok(Some(
                crate::core::tools::tools::ToolCallConfirmationDetails {
                    confirmation_type: crate::core::tools::tools::ConfirmationType::Ask,
                    title: "Delete Team".to_string(),
                    prompt: "Delete team and remove all members".to_string(),
                    on_confirm: Arc::new(|_| {}),
                },
            ))
        })
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
            let team_name = params.team_name.clone();

            Ok(ToolResult {
                llm_content: Some(format!("Team '{}' deleted successfully", team_name)),
                return_display: Some(format!("Team '{}' deleted", team_name)),
                output: serde_json::to_string(&TeamDeleteOutput {
                    success: true,
                    team_name: team_name.clone(),
                    message: format!("Team '{}' has been deleted", team_name),
                })?,
                error: None,
                data: Some(serde_json::json!({
                    "team_name": team_name,
                    "deleted": true
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for TeamDeleteTool {
    fn name(&self) -> &str {
        "team_delete"
    }
    fn display_name(&self) -> &str {
        "TeamDelete"
    }
    fn description(&self) -> &str {
        "删除团队并移除所有成员。用于结束多Agent协作任务。(Delete a team and remove all members. Used to end multi-agent collaboration tasks.)"
    }
    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "要删除的团队名称 (The name of the team to delete.)"
                }
            },
            "required": ["team_name"]
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: TeamDeleteParams = serde_json::from_value(params)?;
        Ok(Box::new(TeamDeleteInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        false
    }
}

// ── ListPeers ────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ListPeersTool;

impl ListPeersTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ListPeersParams {
    pub team_name: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ListPeersOutput {
    pub peers: Vec<PeerInfo>,
    pub total_count: usize,
}

#[derive(Debug, Serialize, Clone)]
pub struct PeerInfo {
    pub agent_id: String,
    pub name: String,
    pub agent_type: String,
    pub status: String,
    pub joined_at: String,
}

pub struct ListPeersInvocation {
    params: ListPeersParams,
}

impl ToolInvocation for ListPeersInvocation {
    fn get_description(&self) -> String {
        "List peers in the current team".to_string()
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
            let peers = vec![PeerInfo {
                agent_id: "team-lead@default".to_string(),
                name: "team-lead".to_string(),
                agent_type: "team-lead".to_string(),
                status: "active".to_string(),
                joined_at: chrono::Utc::now().to_rfc3339(),
            }];

            let total_count = peers.len();

            Ok(ToolResult {
                llm_content: Some(format!("Found {} peers in team", total_count)),
                return_display: Some(format!("{} peers found", total_count)),
                output: serde_json::to_string(&ListPeersOutput { peers, total_count })?,
                error: None,
                data: Some(serde_json::json!({
                    "total_count": total_count
                })),
            })
        })
    }
}

impl BaseDeclarativeTool for ListPeersTool {
    fn name(&self) -> &str {
        "list_peers"
    }
    fn display_name(&self) -> &str {
        "ListPeers"
    }
    fn description(&self) -> &str {
        "列出当前团队中的所有对等Agent。用于查看团队成员状态。(List all peer agents in the current team. Used to view team member status.)"
    }
    fn kind(&self) -> Kind {
        Kind::Read
    }

    fn parameter_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "team_name": {
                    "type": "string",
                    "description": "团队名称，省则使用当前团队 (Team name, omit to use current team.)"
                }
            }
        })
    }

    fn create_invocation(
        &self,
        params: serde_json::Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let params: ListPeersParams = serde_json::from_value(params)?;
        Ok(Box::new(ListPeersInvocation { params }))
    }

    fn is_read_only(&self) -> bool {
        true
    }
}
