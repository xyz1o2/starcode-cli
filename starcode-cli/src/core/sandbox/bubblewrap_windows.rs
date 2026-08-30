//! WSL2 + bubblewrap sandbox runtime for Windows
//!
//! Windows uses WSL2 which provides a full Linux kernel.
//! We run bubblewrap inside WSL2 for sandbox isolation.

use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::config::{FilesystemRule, SandboxConfig};
use super::proxy::NetworkProxy;
use super::runtime::{SandboxError, SandboxResult, SandboxRuntime};

/// Bubblewrap runtime via WSL2 on Windows
pub struct BubblewrapRuntime {
    config: SandboxConfig,
    proxy: Option<NetworkProxy>,
    proxy_port: u16,
}

impl BubblewrapRuntime {
    /// Create a new bubblewrap runtime via WSL2
    pub fn new(config: &SandboxConfig) -> Result<Self, SandboxError> {
        if !Self::is_available() {
            return Err(SandboxError::NotAvailable(
                "WSL2 is not installed. Run: wsl --install".to_string(),
            ));
        }

        Ok(Self {
            config: config.clone(),
            proxy: None,
            proxy_port: 0,
        })
    }

    /// Build WSL2 command with bubblewrap
    fn build_wsl_args(&self, command: &str) -> Vec<String> {
        let mut args = vec!["--".to_string()];

        // Build bubblewrap command
        let mut bwrap_args = vec!["bwrap".to_string()];

        // Filesystem rules
        for rule in &self.config.filesystem {
            match rule {
                FilesystemRule::ReadOnly { path, target } => {
                    let dest = target.as_ref().unwrap_or(path);
                    // Convert Windows path to WSL path
                    let wsl_path = self.windows_to_wsl_path(path);
                    let wsl_dest = self.windows_to_wsl_path(dest);
                    bwrap_args.push("--ro-bind".to_string());
                    bwrap_args.push(wsl_path);
                    bwrap_args.push(wsl_dest);
                }
                FilesystemRule::ReadWrite { path, target } => {
                    let dest = target.as_ref().unwrap_or(path);
                    let wsl_path = self.windows_to_wsl_path(path);
                    let wsl_dest = self.windows_to_wsl_path(dest);
                    bwrap_args.push("--bind".to_string());
                    bwrap_args.push(wsl_path);
                    bwrap_args.push(wsl_dest);
                }
                FilesystemRule::Deny { path } => {
                    let wsl_path = self.windows_to_wsl_path(path);
                    bwrap_args.push("--tmpfs".to_string());
                    bwrap_args.push(wsl_path);
                }
                FilesystemRule::Tmpfs {
                    mount_point,
                    size_mb,
                } => {
                    let wsl_mount = self.windows_to_wsl_path(mount_point);
                    bwrap_args.push("--tmpfs".to_string());
                    bwrap_args.push(wsl_mount);
                    if let Some(size) = size_mb {
                        bwrap_args.push("--size".to_string());
                        bwrap_args.push(format!("{}m", size));
                    }
                }
            }
        }

        // Network isolation
        if !self.config.network.default_action {
            bwrap_args.push("--unshare-net".to_string());
        }

        // Environment variables
        for key in &self.config.env_passthrough {
            if let Ok(value) = std::env::var(key) {
                bwrap_args.push("--setenv".to_string());
                bwrap_args.push(key.clone());
                bwrap_args.push(value);
            }
        }

        for (key, value) in &self.config.env_set {
            bwrap_args.push("--setenv".to_string());
            bwrap_args.push(key.clone());
            bwrap_args.push(value.clone());
        }

        // Working directory
        if let Some(workdir) = &self.config.workdir {
            let wsl_workdir = self.windows_to_wsl_path(workdir);
            bwrap_args.push("--chdir".to_string());
            bwrap_args.push(wsl_workdir);
        }

        // Command to execute
        bwrap_args.push("--".to_string());
        bwrap_args.push("sh".to_string());
        bwrap_args.push("-c".to_string());
        bwrap_args.push(command.to_string());

        args.extend(bwrap_args);
        args
    }

    /// Convert Windows path to WSL path
    fn windows_to_wsl_path(&self, path: &PathBuf) -> String {
        let path_str = path.to_string_lossy();

        // C:\path -> /mnt/c/path
        if path_str.len() >= 2 && path_str.chars().nth(1) == Some(':') {
            let drive = path_str.chars().next().unwrap().to_lowercase();
            let rest = &path_str[2..].replace('\\', "/");
            format!("/mnt/{}{}", drive, rest)
        } else {
            path_str.replace('\\', "/")
        }
    }

    /// Check if bubblewrap is installed in WSL2
    async fn ensure_bubblewrap_installed() -> Result<(), SandboxError> {
        let output = Command::new("wsl")
            .args(["--", "which", "bwrap"])
            .output()
            .await
            .map_err(SandboxError::IoError)?;

        if !output.status.success() {
            // Try to install bubblewrap
            tracing::info!("Installing bubblewrap in WSL2...");
            let install_output = Command::new("wsl")
                .args(["--", "sudo", "apt", "install", "-y", "bubblewrap"])
                .output()
                .await
                .map_err(SandboxError::IoError)?;

            if !install_output.status.success() {
                return Err(SandboxError::NotAvailable(
                    "bubblewrap not installed in WSL2. Run: wsl -e sudo apt install bubblewrap"
                        .to_string(),
                ));
            }
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl SandboxRuntime for BubblewrapRuntime {
    async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        // Ensure bubblewrap is available
        Self::ensure_bubblewrap_installed().await?;

        let args = self.build_wsl_args(command);

        let mut cmd = Command::new("wsl");
        cmd.args(&args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let start = std::time::Instant::now();

        let output = if let Some(secs) = timeout_secs {
            match timeout(Duration::from_secs(secs), cmd.output()).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(SandboxError::IoError(e)),
                Err(_) => return Ok(SandboxResult::timeout(secs)),
            }
        } else {
            cmd.output().await.map_err(SandboxError::IoError)?
        };

        let duration = start.elapsed();
        let mut result = SandboxResult::from(output);
        result.duration = duration;

        Ok(result)
    }

    fn is_available() -> bool {
        // Check WSL2 is available
        if which::which("wsl").is_err() {
            return false;
        }

        // Check bubblewrap in WSL (synchronously)
        std::process::Command::new("wsl")
            .args(["--", "which", "bwrap"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "bubblewrap (via WSL2)"
    }

    async fn prepare(&mut self) -> Result<(), SandboxError> {
        Self::ensure_bubblewrap_installed().await
    }
}
