//! Seatbelt sandbox runtime for macOS
//!
//! Uses Apple's Seatbelt (sandbox-exec) for process isolation:
//! - Filesystem access control
//! - Network access control
//! - Process restrictions
//!
//! Similar to StarCode's sandbox implementation on macOS.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::config::{FilesystemRule, SandboxConfig};
use super::proxy::NetworkProxy;
use super::runtime::{SandboxError, SandboxResult, SandboxRuntime};

/// Seatbelt sandbox runtime
pub struct SeatbeltRuntime {
    config: SandboxConfig,
    proxy: Option<NetworkProxy>,
    proxy_port: u16,
}

impl SeatbeltRuntime {
    /// Create a new Seatbelt runtime
    pub fn new(config: &SandboxConfig) -> Result<Self, SandboxError> {
        if !Self::is_available() {
            return Err(SandboxError::NotAvailable(
                "sandbox-exec is not available on this system".to_string(),
            ));
        }

        Ok(Self {
            config: config.clone(),
            proxy: None,
            proxy_port: 0,
        })
    }

    /// Generate the Seatbelt profile (Sandbox rules language)
    fn generate_profile(&self) -> String {
        let mut profile = String::new();

        // Start with deny all
        profile.push_str("(version 1)\n");
        profile.push_str("(deny default)\n");

        // Allow basic operations
        profile.push_str("(allow process-exec (literal \"/bin/sh\") (literal \"/bin/bash\"))\n");
        profile.push_str("(allow process-fork)\n");
        profile.push_str("(allow signal (target self))\n");
        profile.push_str("(allow sysctl-read)\n");

        // Filesystem rules
        for rule in &self.config.filesystem {
            match rule {
                FilesystemRule::ReadOnly { path, target: _ } => {
                    let path_str = path.to_string_lossy();
                    // Expand ~ to home directory
                    let expanded = shellexpand::tilde(&path_str);
                    profile.push_str(&format!("(allow file-read* (literal \"{}\"))\n", expanded));
                }
                FilesystemRule::ReadWrite { path, target: _ } => {
                    let path_str = path.to_string_lossy();
                    let expanded = shellexpand::tilde(&path_str);
                    profile.push_str(&format!("(allow file-read* (literal \"{}\"))\n", expanded));
                    profile.push_str(&format!("(allow file-write* (literal \"{}\"))\n", expanded));
                }
                FilesystemRule::Deny { path } => {
                    let path_str = path.to_string_lossy();
                    let expanded = shellexpand::tilde(&path_str);
                    profile.push_str(&format!("(deny file* (literal \"{}\"))\n", expanded));
                }
                FilesystemRule::Tmpfs {
                    mount_point,
                    size_mb: _,
                } => {
                    let path_str = mount_point.to_string_lossy();
                    profile.push_str(&format!("(allow file-read* (literal \"{}\"))\n", path_str));
                    profile.push_str(&format!("(allow file-write* (literal \"{}\"))\n", path_str));
                }
            }
        }

        // Network rules
        if self.config.network.default_action == false {
            // Deny network by default
            profile.push_str("(deny network*)\n");

            // Allow localhost if configured
            if self.config.network.allow_localhost {
                profile.push_str("(allow network-outbound (local ip4 \"127.0.0.1\"))\n");
                profile.push_str("(allow network-outbound (local ip6 \"::1\"))\n");
            }

            // Allow specific domains via proxy
            if self.proxy.is_some() {
                // Allow connections to local proxy
                profile.push_str(&format!(
                    "(allow network-outbound (local ip4 \"127.0.0.1:{}\"))\n",
                    self.proxy_port
                ));
            }
        } else {
            // Allow network by default
            profile.push_str("(allow network*)\n");
        }

        // Mach IPC services (required for macOS)
        let mach_services = [
            "com.apple.FontServices.fontservicesserver",
            "com.apple.SecurityServer",
            "com.apple.SystemConfiguration.configd",
            "com.apple.coredumpd",
            "com.apple.launchd",
            "com.apple.trustd",
            "com.apple.trustd.agent",
        ];

        for service in mach_services {
            profile.push_str(&format!(
                "(allow mach-lookup (global-name \"{}\"))\n",
                service
            ));
        }

        profile
    }

    /// Start the network proxy
    async fn start_proxy(&mut self) -> Result<(), SandboxError> {
        if self.config.network.default_action == false
            && !self.config.network.allowed_domains.is_empty()
        {
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
impl SeatbeltRuntime {
    async fn execute_internal(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        let profile = self.generate_profile();

        // Create a temporary file for the profile
        let temp_dir = std::env::temp_dir();
        let profile_path = temp_dir.join(format!("starcode-sandbox-{}.sb", uuid::Uuid::new_v4()));

        {
            let mut file = std::fs::File::create(&profile_path)?;
            file.write_all(profile.as_bytes())?;
        }

        // Build environment with proxy settings
        let mut env_vars: HashMap<String, String> = HashMap::new();

        for key in &self.config.env_passthrough {
            if let Ok(value) = std::env::var(key) {
                env_vars.insert(key.clone(), value);
            }
        }

        for (key, value) in &self.config.env_set {
            env_vars.insert(key.clone(), value.clone());
        }

        // Add proxy environment variables
        if let Some(proxy) = &self.proxy {
            let addr = proxy.address();
            env_vars.insert("HTTP_PROXY".to_string(), format!("http://{}", addr));
            env_vars.insert("HTTPS_PROXY".to_string(), format!("http://{}", addr));
            env_vars.insert("ALL_PROXY".to_string(), format!("socks5://{}", addr));
        }

        // Execute with sandbox-exec
        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-f").arg(&profile_path);
        cmd.arg("--");
        cmd.arg("/bin/sh");
        cmd.arg("-c");
        cmd.arg(command);

        // Set environment variables
        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let start = std::time::Instant::now();

        let output = if let Some(secs) = timeout_secs {
            match timeout(Duration::from_secs(secs), cmd.output()).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => {
                    let _ = std::fs::remove_file(&profile_path);
                    return Err(SandboxError::IoError(e));
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&profile_path);
                    return Ok(SandboxResult::timeout(secs));
                }
            }
        } else {
            cmd.output().await.map_err(|e| {
                let _ = std::fs::remove_file(&profile_path);
                SandboxError::IoError(e)
            })?
        };

        // Cleanup profile file
        let _ = std::fs::remove_file(&profile_path);

        let duration = start.elapsed();
        let mut result = SandboxResult::from(output);
        result.duration = duration;

        Ok(result)
    }
}

#[async_trait::async_trait]
impl SandboxRuntime for SeatbeltRuntime {
    async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        self.execute_internal(command, timeout_secs).await
    }

    fn is_available() -> bool {
        // sandbox-exec is available on all macOS systems
        cfg!(target_os = "macos")
    }

    fn name(&self) -> &'static str {
        "seatbelt"
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
 