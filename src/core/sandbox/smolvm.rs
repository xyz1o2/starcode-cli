//! SmolVM sandbox runtime - microVM isolation
//!
//! SmolVM provides hardware-level virtualization using Firecracker (Linux)
//! or QEMU (macOS) for the strongest isolation boundary.
//!
//! Features:
//! - Sub-second boot time (~572ms)
//! - Hardware-level isolation (independent kernel)
//! - Python SDK integration
//! - Automatic networking and SSH

use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use super::config::SandboxConfig;
use super::runtime::{SandboxError, SandboxResult, SandboxRuntime};

/// SmolVM configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SmolVMConfig {
    /// Memory size in MiB (default: 512)
    #[serde(default = "default_mem_size")]
    pub mem_size_mib: u32,

    /// Disk size in MiB (default: 2048)
    #[serde(default = "default_disk_size")]
    pub disk_size_mib: u32,

    /// Disk mode: "isolated" (per-VM) or "shared"
    #[serde(default = "default_disk_mode")]
    pub disk_mode: String,

    /// Backend: "firecracker" (Linux) or "qemu" (macOS)
    #[serde(default)]
    pub backend: Option<String>,

    /// Python executable path (default: "python3")
    #[serde(default = "default_python")]
    pub python_path: String,

    /// SmolVM package path (optional, for custom installs)
    #[serde(default)]
    pub smolvm_path: Option<PathBuf>,
}

fn default_mem_size() -> u32 {
    512
}
fn default_disk_size() -> u32 {
    2048
}
fn default_disk_mode() -> String {
    "isolated".to_string()
}
fn default_python() -> String {
    "python3".to_string()
}

impl Default for SmolVMConfig {
    fn default() -> Self {
        Self {
            mem_size_mib: default_mem_size(),
            disk_size_mib: default_disk_size(),
            disk_mode: default_disk_mode(),
            backend: None,
            python_path: default_python(),
            smolvm_path: None,
        }
    }
}

/// SmolVM sandbox runtime
pub struct SmolVMRuntime {
    config: SandboxConfig,
    smolvm_config: SmolVMConfig,
    vm_id: Option<String>,
}

impl SmolVMRuntime {
    /// Create a new SmolVM runtime
    pub fn new(config: &SandboxConfig) -> Result<Self, SandboxError> {
        if !Self::is_available() {
            return Err(SandboxError::NotAvailable(
                Self::get_installation_help().join("\n"),
            ));
        }

        let smolvm_config = config.smolvm.clone().unwrap_or_default();

        Ok(Self {
            config: config.clone(),
            smolvm_config,
            vm_id: None,
        })
    }

    /// Get installation help
    pub fn get_installation_help() -> Vec<String> {
        let mut help = vec![
            "📦 SmolVM requires installing Python packages and backend".to_string(),
            "".to_string(),
            "Installation steps:".to_string(),
            "  pip install smolvm".to_string(),
            "".to_string(),
        ];

        #[cfg(target_os = "linux")]
        {
            help.push("Linux (Firecracker):".to_string());
            help.push("  sudo ./scripts/system-setup.sh --configure-runtime".to_string());
            help.push("".to_string());
        }

        #[cfg(target_os = "macos")]
        {
            help.push("macOS (QEMU):".to_string());
            help.push("  brew install qemu".to_string());
            help.push("".to_string());
        }

        help.extend(vec![
            "Verify installation:".to_string(),
            "  smolvm doctor".to_string(),
            "".to_string(),
            "Documentation: https://docs.celesto.ai/smolvm".to_string(),
        ]);

        help
    }

    /// Generate Python script to execute command
    fn generate_exec_script(&self, command: &str) -> String {
        format!(
            r#"
import smolvm
import sys

vm = smolvm.SmolVM(
    mem_size_mib={mem},
    disk_size_mib={disk},
    disk_mode="{disk_mode}"
)
vm.start()

# Set environment variables
{env_vars}

# Execute command
result = vm.run("{cmd}")
print(result.output)

vm.stop()
"#,
            mem = self.smolvm_config.mem_size_mib,
            disk = self.smolvm_config.disk_size_mib,
            disk_mode = self.smolvm_config.disk_mode,
            env_vars = self.generate_env_vars(),
            cmd = command.replace('"', r#"\""#)
        )
    }

    /// Generate environment variable setup
    fn generate_env_vars(&self) -> String {
        let mut lines = Vec::new();

        for key in &self.config.env_passthrough {
            if let Ok(value) = std::env::var(key) {
                lines.push(format!(
                    r#"vm.set_env_vars({{"{}": "{}"}})"#,
                    key,
                    value.replace('"', r#"\""#)
                ));
            }
        }

        for (key, value) in &self.config.env_set {
            lines.push(format!(
                r#"vm.set_env_vars({{"{}": "{}"}})"#,
                key,
                value.replace('"', r#"\""#)
            ));
        }

        lines.join("\n")
    }
}

#[async_trait::async_trait]
impl SandboxRuntime for SmolVMRuntime {
    async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        let script = self.generate_exec_script(command);

        let mut cmd = Command::new(&self.smolvm_config.python_path);
        cmd.args(["-c", &script]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let start = std::time::Instant::now();

        let output = if let Some(secs) = timeout_secs {
            // SmolVM needs extra time for VM boot
            let total_secs = secs + 10; // Add 10s buffer for VM lifecycle
            match timeout(Duration::from_secs(total_secs), cmd.output()).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(SandboxError::IoError(e)),
                Err(_) => return Ok(SandboxResult::timeout(secs)),
            }
        } else {
            // Default 5 minute timeout for SmolVM
            match timeout(Duration::from_secs(300), cmd.output()).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(SandboxError::IoError(e)),
                Err(_) => return Ok(SandboxResult::timeout(300)),
            }
        };

        let duration = start.elapsed();
        let mut result = SandboxResult::from(output);
        result.duration = duration;

        Ok(result)
    }

    fn is_available() -> bool {
        // Check Python and smolvm package synchronously
        std::process::Command::new("python3")
            .args(["-c", "import smolvm"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        #[cfg(target_os = "linux")]
        {
            "SmolVM (Firecracker)"
        }
        #[cfg(target_os = "macos")]
        {
            "SmolVM (QEMU)"
        }
    }

    async fn prepare(&mut self) -> Result<(), SandboxError> {
        // Verify smolvm is installed
        if !Self::is_available() {
            return Err(SandboxError::NotAvailable(
                "SmolVM not installed. Run: pip install smolvm".to_string(),
            ));
        }

        // Run smolvm doctor to verify backend
        let output = Command::new("smolvm")
            .args(["doctor"])
            .output()
            .await
            .map_err(SandboxError::IoError)?;

        if !output.status.success() {
            tracing::warn!(
                "SmolVM doctor check failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    async fn cleanup(&mut self) -> Result<(), SandboxError> {
        // SmolVM auto-cleans on stop, but we can force cleanup if needed
        if let Some(vm_id) = &self.vm_id {
            let _ = Command::new("smolvm")
                .args(["delete", vm_id])
                .output()
                .await;
        }
        Ok(())
    }
}
