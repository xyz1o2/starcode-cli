//! Bubblewrap sandbox runtime for Linux
//!
//! Uses bubblewrap (bwrap) for lightweight namespace isolation:
//! - PID namespace isolation
//! - Network namespace isolation (with optional proxy)
//! - Filesystem isolation with bind mounts
//!
//! Similar to StarCode's sandbox implementation on Linux.

use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::config::{FilesystemRule, SandboxConfig};
use super::proxy::NetworkProxy;
use super::runtime::{SandboxError, SandboxResult, SandboxRuntime};

/// Bubblewrap sandbox runtime
pub struct BubblewrapRuntime {
    config: SandboxConfig,
    proxy: Option<NetworkProxy>,
    proxy_port: u16,
}

impl BubblewrapRuntime {
    /// Create a new bubblewrap runtime
    pub fn new(config: &SandboxConfig) -> Result<Self, SandboxError> {
        if !Self::is_available() {
            return Err(SandboxError::NotAvailable(
                "bubblewrap (bwrap) is not installed. Install with: apt install bubblewrap"
                    .to_string(),
            ));
        }

        Ok(Self {
            config: config.clone(),
            proxy: None,
            proxy_port: 0,
        })
    }

    /// Check if bwrap is available
    fn find_bwrap() -> Option<PathBuf> {
        // Check common locations
        let paths = ["/usr/bin/bwrap", "/usr/local/bin/bwrap", "bwrap"];
        for path in paths {
            let result = if path.starts_with('/') {
                std::fs::metadata(path).is_ok()
            } else {
                which::which(path).is_ok()
            };
            if result {
                return Some(PathBuf::from(path));
            }
        }
        None
    }

    /// Build bwrap arguments from configuration
    fn build_args(&self, command: &str) -> Vec<String> {
        let mut args = Vec::new();

        // Filesystem rules
        for rule in &self.config.filesystem {
            match rule {
                FilesystemRule::ReadOnly { path, target } => {
                    let dest = target.as_ref().unwrap_or(path);
                    args.push("--ro-bind".to_string());
                    args.push(path.to_string_lossy().to_string());
                    args.push(dest.to_string_lossy().to_string());
                }
                FilesystemRule::ReadWrite { path, target } => {
                    let dest = target.as_ref().unwrap_or(path);
                    args.push("--bind".to_string());
                    args.push(path.to_string_lossy().to_string());
                    args.push(dest.to_string_lossy().to_string());
                }
                FilesystemRule::Deny { path } => {
                    // Create a tmpfs overlay to deny access
                    args.push("--tmpfs".to_string());
                    args.push(path.to_string_lossy().to_string());
                }
                FilesystemRule::Tmpfs {
                    mount_point,
                    size_mb,
                } => {
                    args.push("--tmpfs".to_string());
                    args.push(mount_point.to_string_lossy().to_string());
                    if let Some(size) = size_mb {
                        args.push("--size".to_string());
                        args.push(format!("{}M", size));
                    }
                }
            }
        }

        // Network isolation
        if self.config.network.default_action == false {
            // Deny network by default - unshare network namespace
            args.push("--unshare-net".to_string());
        }

        // PID namespace
        args.push("--unshare-pid".to_string());

        // Die with parent
        args.push("--die-with-parent".to_string());

        // Proc filesystem
        args.push("--proc".to_string());
        args.push("/proc".to_string());

        // Dev filesystem (minimal)
        args.push("--dev".to_string());
        args.push("/dev".to_string());

        // Working directory
        if let Some(workdir) = &self.config.workdir {
            args.push("--chdir".to_string());
            args.push(workdir.to_string_lossy().to_string());
        }

        // Environment variables
        for key in &self.config.env_passthrough {
            if let Ok(value) = std::env::var(key) {
                args.push("--setenv".to_string());
                args.push(key.clone());
                args.push(value);
            }
        }

        for (key, value) in &self.config.env_set {
            args.push("--setenv".to_string());
            args.push(key.clone());
            args.push(value.clone());
        }

        // Set proxy environment variables if proxy is active
        if let Some(proxy) = &self.proxy {
            let addr = proxy.address();
            args.push("--setenv".to_string());
            args.push("HTTP_PROXY".to_string());
            args.push(format!("http://{}", addr));
            args.push("--setenv".to_string());
            args.push("HTTPS_PROXY".to_string());
            args.push(format!("http://{}", addr));
            args.push("--setenv".to_string());
            args.push("ALL_PROXY".to_string());
            args.push(format!("socks5://{}", addr));
        }

        // Command to execute
        args.push("--".to_string());
        args.push("/bin/sh".to_string());
        args.push("-c".to_string());
        args.push(command.to_string());

        args
    }

    /// Start the network proxy
    async fn start_proxy(&mut self) -> Result<(), SandboxError> {
        if self.config.network.default_action == false
            && !self.config.network.allowed_domains.is_empty()
        {
            // Find an available port
            self.proxy_port = find_available_port().await?;

            let proxy = NetworkProxy::new(&self.config.network, self.proxy_port);
            proxy
                .start()
                .await
                .map_err(|e| SandboxError::ProxyError(e.to_string()))?;
            self.proxy = Some(proxy);
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
        let bwrap_path = Self::find_bwrap()
            .ok_or_else(|| SandboxError::NotAvailable("bwrap not found".to_string()))?;

        let args = self.build_args(command);

        let mut cmd = Command::new(&bwrap_path);
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
        Self::find_bwrap().is_some()
    }

    fn name(&self) -> &'static str {
        "bubblewrap"
    }

    async fn prepare(&mut self) -> Result<(), SandboxError> {
        self.start_proxy().await
    }

    async fn cleanup(&mut self) -> Result<(), SandboxError> {
        if let Some(proxy) = &self.proxy {
            proxy.stop().await;
        }
        Ok(())
    }
}

/// Find an available port for the proxy
async fn find_available_port() -> Result<u16, SandboxError> {
    use tokio::net::TcpListener;

    // Try ports starting from 38080
    for port in 38080..40000 {
        let addr = format!("127.0.0.1:{}", port);
        if TcpListener::bind(&addr).await.is_ok() {
            return Ok(port);
        }
    }

    Err(SandboxError::ProxyError(
        "No available port found".to_string(),
    ))
}
