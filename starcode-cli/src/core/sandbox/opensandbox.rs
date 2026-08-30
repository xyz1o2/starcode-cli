//! OpenSandbox runtime integration
//!
//! Provides enterprise-grade sandbox isolation through OpenSandbox server.
//! Requires OpenSandbox server to be running.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

use super::config::{OpenSandboxConfig, SandboxConfig};
use super::runtime::{SandboxError, SandboxResult, SandboxRuntime};

/// OpenSandbox API client
pub struct OpenSandboxRuntime {
    config: SandboxConfig,
    opensandbox_config: OpenSandboxConfig,
    sandbox_id: Option<String>,
    client: reqwest::Client,
}

impl OpenSandboxRuntime {
    /// Create a new OpenSandbox runtime
    pub fn new(config: &SandboxConfig) -> Result<Self, SandboxError> {
        let opensandbox_config = config.opensandbox.clone().ok_or_else(|| {
            SandboxError::ConfigError("OpenSandbox configuration not provided".to_string())
        })?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| SandboxError::OpenSandboxError(e.to_string()))?;

        Ok(Self {
            config: config.clone(),
            opensandbox_config,
            sandbox_id: None,
            client,
        })
    }

    /// Create a new sandbox on the server
    async fn create_sandbox(&mut self) -> Result<String, SandboxError> {
        let mut request = CreateSandboxRequest {
            ttl: self.opensandbox_config.ttl_secs,
            ..Default::default()
        };

        // Add filesystem rules
        for rule in &self.config.filesystem {
            match rule {
                super::config::FilesystemRule::ReadWrite { path, target: _ } => {
                    request.volumes.push(VolumeMount {
                        host_path: path.to_string_lossy().to_string(),
                        container_path: path.to_string_lossy().to_string(),
                        read_only: false,
                    });
                }
                super::config::FilesystemRule::ReadOnly { path, target: _ } => {
                    request.volumes.push(VolumeMount {
                        host_path: path.to_string_lossy().to_string(),
                        container_path: path.to_string_lossy().to_string(),
                        read_only: true,
                    });
                }
                _ => {}
            }
        }

        // Add network policy
        if !self.config.network.allowed_domains.is_empty() {
            request.network_policy = Some(NetworkPolicy {
                default_action: if self.config.network.default_action {
                    "allow"
                } else {
                    "deny"
                }
                .to_string(),
                egress: self
                    .config
                    .network
                    .allowed_domains
                    .iter()
                    .map(|d| EgressRule {
                        action: "allow".to_string(),
                        target: d.clone(),
                    })
                    .collect(),
            });
        }

        let mut builder = self
            .client
            .post(format!(
                "{}/v1/sandboxes",
                self.opensandbox_config.server_url
            ))
            .json(&request);

        if let Some(api_key) = &self.opensandbox_config.api_key {
            builder = builder.header("OPEN-SANDBOX-API-KEY", api_key);
        }

        let response = builder
            .send()
            .await
            .map_err(|e| SandboxError::OpenSandboxError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SandboxError::OpenSandboxError(format!(
                "Failed to create sandbox: {}",
                error_text
            )));
        }

        let result: CreateSandboxResponse = response
            .json()
            .await
            .map_err(|e| SandboxError::OpenSandboxError(e.to_string()))?;

        self.sandbox_id = Some(result.sandbox_id.clone());
        Ok(result.sandbox_id)
    }

    /// Execute a command in the sandbox
    async fn execute_in_sandbox(
        &self,
        sandbox_id: &str,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        let request = ExecuteRequest {
            command: command.to_string(),
            timeout: timeout_secs,
        };

        let mut builder = self
            .client
            .post(format!(
                "{}/v1/sandboxes/{}/execute",
                self.opensandbox_config.server_url, sandbox_id
            ))
            .json(&request);

        if let Some(api_key) = &self.opensandbox_config.api_key {
            builder = builder.header("OPEN-SANDBOX-API-KEY", api_key);
        }

        let response = builder
            .send()
            .await
            .map_err(|e| SandboxError::OpenSandboxError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(SandboxError::OpenSandboxError(format!(
                "Execute failed: {}",
                error_text
            )));
        }

        let result: ExecuteResponse = response
            .json()
            .await
            .map_err(|e| SandboxError::OpenSandboxError(e.to_string()))?;

        Ok(SandboxResult {
            success: result.exit_code == 0,
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: Some(result.exit_code),
            duration: Duration::from_millis(result.duration_ms.unwrap_or(0)),
            metadata: HashMap::new(),
        })
    }

    /// Delete the sandbox
    async fn delete_sandbox(&self, sandbox_id: &str) -> Result<(), SandboxError> {
        let mut builder = self.client.delete(format!(
            "{}/v1/sandboxes/{}",
            self.opensandbox_config.server_url, sandbox_id
        ));

        if let Some(api_key) = &self.opensandbox_config.api_key {
            builder = builder.header("OPEN-SANDBOX-API-KEY", api_key);
        }

        let _ = builder
            .send()
            .await
            .map_err(|e| SandboxError::OpenSandboxError(e.to_string()))?;

        Ok(())
    }

    /// Check if OpenSandbox server is reachable
    async fn check_server_available(&self) -> bool {
        let result = self
            .client
            .get(format!("{}/health", self.opensandbox_config.server_url))
            .send()
            .await;

        result.map(|r| r.status().is_success()).unwrap_or(false)
    }
}

#[async_trait::async_trait]
impl SandboxRuntime for OpenSandboxRuntime {
    async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        let sandbox_id = self
            .sandbox_id
            .as_ref()
            .ok_or_else(|| SandboxError::OpenSandboxError("Sandbox not created".to_string()))?;

        self.execute_in_sandbox(sandbox_id, command, timeout_secs)
            .await
    }

    fn is_available() -> bool {
        // This is a placeholder - actual availability is checked asynchronously
        true
    }

    fn name(&self) -> &'static str {
        "opensandbox"
    }

    async fn prepare(&mut self) -> Result<(), SandboxError> {
        if !self.check_server_available().await {
            return Err(SandboxError::NotAvailable(format!(
                "OpenSandbox server not reachable at {}",
                self.opensandbox_config.server_url
            )));
        }

        self.create_sandbox().await?;
        Ok(())
    }

    async fn cleanup(&mut self) -> Result<(), SandboxError> {
        if let Some(sandbox_id) = &self.sandbox_id {
            self.delete_sandbox(sandbox_id).await?;
            self.sandbox_id = None;
        }
        Ok(())
    }
}

// API types

#[derive(Debug, Serialize, Default)]
struct CreateSandboxRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    volumes: Vec<VolumeMount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    network_policy: Option<NetworkPolicy>,
}

#[derive(Debug, Serialize)]
struct VolumeMount {
    host_path: String,
    container_path: String,
    read_only: bool,
}

#[derive(Debug, Serialize)]
struct NetworkPolicy {
    default_action: String,
    egress: Vec<EgressRule>,
}

#[derive(Debug, Serialize)]
struct EgressRule {
    action: String,
    target: String,
}

#[derive(Debug, Deserialize)]
struct CreateSandboxResponse {
    sandbox_id: String,
    #[serde(rename = "status")]
    _status: String,
}

#[derive(Debug, Serialize)]
struct ExecuteRequest {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct ExecuteResponse {
    exit_code: i32,
    stdout: String,
    stderr: String,
    duration_ms: Option<u64>,
}
