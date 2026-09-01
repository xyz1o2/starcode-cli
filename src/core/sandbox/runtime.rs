//! Sandbox runtime trait and common types

use std::process::Output;
use std::time::Duration;
use thiserror::Error;

/// Sandbox execution error
#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("Sandbox not available: {0}")]
    NotAvailable(String),

    #[error("Failed to create sandbox: {0}")]
    CreationFailed(String),

    #[error("Failed to execute command: {0}")]
    ExecutionFailed(String),

    #[error("Command timed out after {0} seconds")]
    Timeout(u64),

    #[error("Network proxy error: {0}")]
    ProxyError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Unsupported platform: {0}")]
    UnsupportedPlatform(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("OpenSandbox error: {0}")]
    OpenSandboxError(String),
}

/// Result of sandbox execution
#[derive(Debug)]
pub struct SandboxResult {
    /// Whether the command succeeded
    pub success: bool,

    /// Standard output
    pub stdout: String,

    /// Standard error
    pub stderr: String,

    /// Exit code
    pub exit_code: Option<i32>,

    /// Execution duration
    pub duration: Duration,

    /// Sandbox-specific metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl SandboxResult {
    /// Create a successful result
    pub fn success(stdout: String, stderr: String, exit_code: i32) -> Self {
        Self {
            success: true,
            stdout,
            stderr,
            exit_code: Some(exit_code),
            duration: Duration::ZERO,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create a failed result
    pub fn failure(stderr: String, exit_code: Option<i32>) -> Self {
        Self {
            success: false,
            stdout: String::new(),
            stderr,
            exit_code,
            duration: Duration::ZERO,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Create a timeout result
    pub fn timeout(timeout_secs: u64) -> Self {
        Self {
            success: false,
            stdout: String::new(),
            stderr: format!("Command timed out after {} seconds", timeout_secs),
            exit_code: Some(-1),
            duration: Duration::from_secs(timeout_secs),
            metadata: std::collections::HashMap::new(),
        }
    }
}

impl From<Output> for SandboxResult {
    fn from(output: Output) -> Self {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code();

        Self {
            success: output.status.success(),
            stdout,
            stderr,
            exit_code,
            duration: Duration::ZERO,
            metadata: std::collections::HashMap::new(),
        }
    }
}

/// Sandbox runtime trait - platform-specific implementations
#[async_trait::async_trait]
pub trait SandboxRuntime: Send + Sync {
    /// Execute a command in the sandbox
    async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError>;

    /// Check if the runtime is available on this system
    fn is_available() -> bool
    where
        Self: Sized;

    /// Get the runtime name
    fn name(&self) -> &'static str;

    /// Prepare the sandbox environment (called once before first execution)
    async fn prepare(&mut self) -> Result<(), SandboxError> {
        Ok(())
    }

    /// Cleanup sandbox resources (called on shutdown)
    async fn cleanup(&mut self) -> Result<(), SandboxError> {
        Ok(())
    }
}

/// No-op runtime for when sandboxing is disabled
pub struct NoopRuntime;

impl NoopRuntime {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl SandboxRuntime for NoopRuntime {
    async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        use tokio::process::Command;
        use tokio::time::{timeout, Duration};

        #[cfg(unix)]
        let output = {
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command);

            if let Some(secs) = timeout_secs {
                match timeout(Duration::from_secs(secs), cmd.output()).await {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => return Err(SandboxError::IoError(e)),
                    Err(_) => return Ok(SandboxResult::timeout(secs)),
                }
            } else {
                cmd.output().await.map_err(SandboxError::IoError)?
            }
        };

        #[cfg(windows)]
        let output = {
            let mut cmd = Command::new("powershell.exe");
            cmd.arg("-NoProfile").arg("-Command").arg(command);

            if let Some(secs) = timeout_secs {
                match timeout(Duration::from_secs(secs), cmd.output()).await {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => return Err(SandboxError::IoError(e)),
                    Err(_) => return Ok(SandboxResult::timeout(secs)),
                }
            } else {
                cmd.output().await.map_err(SandboxError::IoError)?
            }
        };

        Ok(SandboxResult::from(output))
    }

    fn is_available() -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

impl Default for NoopRuntime {
    fn default() -> Self {
        Self::new()
    }
}
