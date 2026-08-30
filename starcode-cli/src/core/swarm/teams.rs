//! Swarm 团队系统
//!
//! 对标 Claude Code 的 Agent Teams / Swarm：
//! - TeamFile 定义
//! - Teammate 生命周期
//! - 团队协作协议

use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::mailbox::{MailboxManager, MailboxMessage, MessageType};

/// 团队定义文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamFile {
    pub name: String,
    pub description: String,
    pub members: Vec<TeammateDefinition>,
    pub shared_context: Option<String>,
    pub max_concurrent: usize,
}

/// Teammate 定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateDefinition {
    pub name: String,
    pub role: String,
    pub capabilities: Vec<String>,
    pub system_prompt: Option<String>,
    pub tools: Option<Vec<String>>,
}

/// Teammate 实例
#[derive(Debug, Clone)]
pub struct TeammateInstance {
    pub id: String,
    pub definition: TeammateDefinition,
    pub status: TeammateStatus,
    pub current_task: Option<String>,
    pub messages_processed: usize,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeammateStatus {
    Initializing,
    Idle,
    Working,
    WaitingApproval,
    Failed,
    Completed,
}

/// Swarm 管理器
pub struct SwarmManager {
    teams: HashMap<String, TeamInstance>,
    mailbox: MailboxManager,
}

/// 团队实例
#[derive(Debug, Clone)]
pub struct TeamInstance {
    pub definition: TeamFile,
    pub members: Vec<TeammateInstance>,
    pub status: TeamStatus,
    pub round: usize,
    pub max_rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamStatus {
    Created,
    Running,
    Paused,
    Completed,
    Failed,
}

impl SwarmManager {
    pub fn new() -> Self {
        Self {
            teams: HashMap::new(),
            mailbox: MailboxManager::new(),
        }
    }

    pub fn create_team(&mut self, team_file: TeamFile) -> Result<String, String> {
        let team_name = team_file.name.clone();

        let members: Vec<TeammateInstance> = team_file
            .members
            .iter()
            .map(|def| TeammateInstance {
                id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
                definition: def.clone(),
                status: TeammateStatus::Initializing,
                current_task: None,
                messages_processed: 0,
                created_at: now_secs(),
            })
            .collect();

        let instance = TeamInstance {
            definition: team_file,
            members,
            status: TeamStatus::Created,
            round: 0,
            max_rounds: 20,
        };

        self.teams.insert(team_name.clone(), instance);
        Ok(team_name)
    }

    pub fn start_team(&mut self, team_name: &str) -> Result<(), String> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| format!("Team '{}' not found", team_name))?;

        team.status = TeamStatus::Running;
        for member in &mut team.members {
            member.status = TeammateStatus::Idle;
        }
        Ok(())
    }

    pub fn assign_task(
        &mut self,
        team_name: &str,
        member_name: &str,
        task: String,
    ) -> Result<(), String> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| format!("Team '{}' not found", team_name))?;

        let member = team
            .members
            .iter_mut()
            .find(|m| m.definition.name == member_name)
            .ok_or_else(|| format!("Member '{}' not found in team '{}'", member_name, team_name))?;

        if member.status != TeammateStatus::Idle {
            return Err(format!("Member '{}' is not idle (status: {:?})", member_name, member.status));
        }

        member.status = TeammateStatus::Working;
        member.current_task = Some(task.clone());

        // 通过 mailbox 投递任务
        let msg = MailboxMessage {
            id: uuid::Uuid::new_v4().to_string(),
            from: "swarm_manager".to_string(),
            to: member_name.to_string(),
            message_type: MessageType::TaskAssignment,
            content: task,
            summary: Some(format!("Task assigned in round {}", team.round)),
            timestamp_ms: now_ms(),
            read: false,
            color: None,
        };

        if let Err(e) = self.mailbox.send_message(team_name, member_name, msg) {
            log::warn!("Failed to send mailbox message: {}", e);
        }

        Ok(())
    }

    pub fn receive_result(
        &mut self,
        team_name: &str,
        member_name: &str,
        _result: Value,
    ) -> Result<(), String> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| format!("Team '{}' not found", team_name))?;

        let member = team
            .members
            .iter_mut()
            .find(|m| m.definition.name == member_name)
            .ok_or_else(|| format!("Member '{}' not found", member_name))?;

        member.status = TeammateStatus::Completed;
        member.current_task = None;
        member.messages_processed += 1;
        Ok(())
    }

    pub fn advance_round(&mut self, team_name: &str) -> Result<bool, String> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| format!("Team '{}' not found", team_name))?;

        if team.round >= team.max_rounds {
            team.status = TeamStatus::Completed;
            return Ok(false);
        }

        team.round += 1;
        for member in &mut team.members {
            if member.status == TeammateStatus::Completed {
                member.status = TeammateStatus::Idle;
            }
        }
        Ok(true)
    }

    pub fn team_status(&self, team_name: &str) -> Option<Value> {
        self.teams.get(team_name).map(|team| {
            json!({
                "name": team.definition.name,
                "status": format!("{:?}", team.status),
                "round": team.round,
                "max_rounds": team.max_rounds,
                "members": team.members.iter().map(|m| json!({
                    "name": m.definition.name,
                    "role": m.definition.role,
                    "status": format!("{:?}", m.status),
                    "current_task": m.current_task,
                    "messages_processed": m.messages_processed,
                })).collect::<Vec<_>>(),
            })
        })
    }

    pub fn list_teams(&self) -> Vec<String> {
        self.teams.keys().cloned().collect()
    }

    pub fn stop_team(&mut self, team_name: &str) -> Result<(), String> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| format!("Team '{}' not found", team_name))?;

        team.status = TeamStatus::Completed;
        for member in &mut team.members {
            member.status = TeammateStatus::Completed;
            member.current_task = None;
        }
        Ok(())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn load_team_file(path: &std::path::Path) -> Result<TeamFile, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read team file: {}", e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse team file: {}", e))
}

pub fn validate_team_file(team: &TeamFile) -> Result<(), String> {
    if team.name.is_empty() {
        return Err("Team name cannot be empty".to_string());
    }
    if team.members.is_empty() {
        return Err("Team must have at least one member".to_string());
    }
    if team.max_concurrent == 0 {
        return Err("max_concurrent must be > 0".to_string());
    }

    let mut names = std::collections::HashSet::new();
    for member in &team.members {
        if !names.insert(&member.name) {
            return Err(format!("Duplicate member name: {}", member.name));
        }
    }
    Ok(())
}
