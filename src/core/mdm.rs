use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdmManager {
    pub enrolled: bool,
    pub server_url: Option<String>,
    pub policies: Vec<MdmPolicy>,
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MdmPolicy {
    pub id: String,
    pub name: String,
    pub policy_type: PolicyType,
    pub value: Value,
    pub enforced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PolicyType {
    AllowedTools,
    BlockedTools,
    MaxTokens,
    AllowedModels,
    BlockedModels,
    SessionTimeout,
    RequireApproval,
}

impl MdmManager {
    pub fn new() -> Self {
        Self {
            enrolled: false,
            server_url: None,
            policies: Vec::new(),
            device_id: Self::generate_device_id(),
        }
    }

    fn generate_device_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    pub fn enroll(&mut self, server_url: &str) -> Result<(), String> {
        self.server_url = Some(server_url.to_string());
        self.enrolled = true;
        Ok(())
    }

    pub fn unenroll(&mut self) {
        self.enrolled = false;
        self.server_url = None;
        self.policies.clear();
    }

    pub async fn sync_policies(&mut self) -> Result<(), String> {
        if !self.enrolled {
            return Ok(());
        }

        let server_url = self
            .server_url
            .as_ref()
            .ok_or("Not enrolled in MDM server")?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/api/v1/policies/{}", server_url, self.device_id))
            .send()
            .await
            .map_err(|e| format!("Failed to sync policies: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Failed to sync policies: HTTP {}",
                response.status()
            ));
        }

        let policies: Vec<MdmPolicy> = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse policies: {}", e))?;

        self.policies = policies;
        Ok(())
    }

    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        for policy in &self.policies {
            if !policy.enforced {
                continue;
            }

            match &policy.policy_type {
                PolicyType::AllowedTools => {
                    if let Some(allowed) = policy.value.as_array() {
                        if !allowed.iter().any(|v| v.as_str() == Some(tool_name)) {
                            return false;
                        }
                    }
                }
                PolicyType::BlockedTools => {
                    if let Some(blocked) = policy.value.as_array() {
                        if blocked.iter().any(|v| v.as_str() == Some(tool_name)) {
                            return false;
                        }
                    }
                }
                _ => {}
            }
        }
        true
    }

    pub fn get_max_tokens(&self) -> Option<u64> {
        for policy in &self.policies {
            if !policy.enforced {
                continue;
            }

            if let PolicyType::MaxTokens = policy.policy_type {
                return policy.value.as_u64();
            }
        }
        None
    }

    pub fn is_model_allowed(&self, model: &str) -> bool {
        for policy in &self.policies {
            if !policy.enforced {
                continue;
            }

            match &policy.policy_type {
                PolicyType::AllowedModels => {
                    if let Some(allowed) = policy.value.as_array() {
                        if !allowed.iter().any(|v| v.as_str() == Some(model)) {
                            return false;
                        }
                    }
                }
                PolicyType::BlockedModels => {
                    if let Some(blocked) = policy.value.as_array() {
                        if blocked.iter().any(|v| v.as_str() == Some(model)) {
                            return false;
                        }
                    }
                }
                _ => {}
            }
        }
        true
    }

    pub fn get_session_timeout(&self) -> Option<u64> {
        for policy in &self.policies {
            if !policy.enforced {
                continue;
            }

            if let PolicyType::SessionTimeout = policy.policy_type {
                return policy.value.as_u64();
            }
        }
        None
    }

    pub fn requires_approval(&self) -> bool {
        for policy in &self.policies {
            if !policy.enforced {
                continue;
            }

            if let PolicyType::RequireApproval = policy.policy_type {
                return policy.value.as_bool().unwrap_or(false);
            }
        }
        false
    }

    pub fn get_status(&self) -> String {
        if !self.enrolled {
            return "Not enrolled in MDM".to_string();
        }

        let mut status = format!(
            "Enrolled in MDM\nServer: {}\nDevice ID: {}\nPolicies: {}",
            self.server_url.as_deref().unwrap_or("unknown"),
            self.device_id,
            self.policies.len()
        );

        if !self.policies.is_empty() {
            status.push_str("\n\nActive Policies:");
            for policy in &self.policies {
                status.push_str(&format!(
                    "\n  - {} ({}): {}",
                    policy.name,
                    if policy.enforced {
                        "enforced"
                    } else {
                        "advisory"
                    },
                    policy.value
                ));
            }
        }

        status
    }
}

impl Default for MdmManager {
    fn default() -> Self {
        Self::new()
    }
}
