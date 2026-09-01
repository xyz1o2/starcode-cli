//! Sandbox module for command execution isolation
//!
//! Provides a hybrid sandbox solution:
//! - Linux: bubblewrap (namespace isolation)
//! - macOS: Seatbelt (sandbox-exec)
//! - Windows: WSL2 + bubblewrap (via WSL2 Linux kernel)
//! - Optional: OpenSandbox for enterprise-grade isolation
//!
//! # Architecture
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │           SandboxRuntime (trait)            │
//! ├─────────────────────────────────────────────┤
//! │  Linux: BubblewrapRuntime                   │
//! │  macOS: SeatbeltRuntime                     │
//! │  Windows: WSL2 + Bubblewrap (same as Linux) │
//! │  Any: OpenSandboxRuntime (optional)         │
//! └─────────────────────────────────────────────┘
//! ```

pub mod config;
pub mod network;
pub mod proxy;
pub mod runtime;

#[cfg(target_os = "linux")]
pub mod bubblewrap;

#[cfg(target_os = "macos")]
pub mod seatbelt;

#[cfg(target_os = "windows")]
pub mod bubblewrap_windows;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod smolvm;

// Windows uses WSL2 which is Linux, so we reuse bubblewrap
// Docker Desktop on Windows also uses WSL2 as backend

pub mod opensandbox;

pub use config::{SandboxConfig, SandboxMode};
pub use runtime::{SandboxError, SandboxResult, SandboxRuntime};

/// Sandbox manager for executing commands in isolated environments
pub struct SandboxManager {
    config: SandboxConfig,
    runtime: Box<dyn SandboxRuntime>,
}

impl SandboxManager {
    /// Create a new sandbox manager with the given configuration
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        let runtime = Self::create_runtime(&config)?;
        Ok(Self { config, runtime })
    }

    /// Create appropriate runtime based on configuration and platform
    fn create_runtime(config: &SandboxConfig) -> Result<Box<dyn SandboxRuntime>, SandboxError> {
        match config.mode {
            #[cfg(target_os = "linux")]
            SandboxMode::Bubblewrap => Ok(Box::new(bubblewrap::BubblewrapRuntime::new(config)?)),
            #[cfg(target_os = "macos")]
            SandboxMode::Seatbelt => Ok(Box::new(seatbelt::SeatbeltRuntime::new(config)?)),
            #[cfg(target_os = "windows")]
            SandboxMode::Docker => {
                // Windows uses WSL2 which provides Linux environment
                // We check for WSL2 and use bubblewrap inside it
                if Self::is_wsl2_available() {
                    Ok(Box::new(bubblewrap_windows::BubblewrapRuntime::new(
                        config,
                    )?))
                } else {
                    Err(SandboxError::NotAvailable(
                        Self::get_installation_help().join("\n"),
                    ))
                }
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            SandboxMode::SmolVM => Ok(Box::new(smolvm::SmolVMRuntime::new(config)?)),
            SandboxMode::OpenSandbox => Ok(Box::new(opensandbox::OpenSandboxRuntime::new(config)?)),
            SandboxMode::None => Ok(Box::new(runtime::NoopRuntime::new())),
            #[allow(unreachable_patterns)]
            _ => Err(SandboxError::UnsupportedPlatform(format!(
                "Sandbox mode {:?} not supported on this platform",
                config.mode
            ))),
        }
    }

    /// Execute a command in the sandbox
    pub async fn execute(
        &self,
        command: &str,
        timeout_secs: Option<u64>,
    ) -> Result<SandboxResult, SandboxError> {
        self.runtime.execute(command, timeout_secs).await
    }

    /// Check if WSL2 is available (Windows only)
    #[cfg(target_os = "windows")]
    fn is_wsl2_available() -> bool {
        which::which("wsl").is_ok()
    }

    /// Check if sandbox is available on this platform
    pub fn is_available() -> bool {
        #[cfg(target_os = "linux")]
        {
            bubblewrap::BubblewrapRuntime::is_available()
        }
        #[cfg(target_os = "macos")]
        {
            seatbelt::SeatbeltRuntime::is_available()
        }
        #[cfg(target_os = "windows")]
        {
            // Windows needs WSL2 which provides Linux environment
            Self::is_wsl2_available() && bubblewrap_windows::BubblewrapRuntime::is_available()
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            false
        }
    }

    /// Get installation help for current platform
    pub fn get_installation_help() -> Vec<String> {
        #[cfg(target_os = "linux")]
        {
            vec![
                "📦 Linux sandbox requires bubblewrap".to_string(),
                "".to_string(),
                "Installation commands:".to_string(),
                "  sudo apt install bubblewrap  # Debian/Ubuntu".to_string(),
                "  sudo dnf install bubblewrap  # Fedora".to_string(),
                "  sudo pacman -S bubblewrap    # Arch".to_string(),
            ]
        }
        #[cfg(target_os = "macos")]
        {
            vec![
                "📦 macOS sandbox uses the built-in sandbox-exec".to_string(),
                "".to_string(),
                "No additional installation required, system is already supported.".to_string(),
            ]
        }
        #[cfg(target_os = "windows")]
        {
            vec![
                "📦 Windows sandbox requires WSL2 (Windows Subsystem for Linux)".to_string(),
                "".to_string(),
                "WSL2 provides a complete Linux kernel with bubblewrap isolation support."
                    .to_string(),
                "".to_string(),
                "Installation steps:".to_string(),
                "  1. Open PowerShell as Administrator".to_string(),
                "  2. Run: wsl --install".to_string(),
                "  3. Restart your computer".to_string(),
                "  4. Open WSL terminal and run: sudo apt install bubblewrap".to_string(),
                "".to_string(),
                "Or use winget:".to_string(),
                "  winget install Microsoft.WSL".to_string(),
                "".to_string(),
                "Note: Docker Desktop also uses WSL2, it will work after installation.".to_string(),
            ]
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            vec!["Sandbox is not supported on this platform".to_string()]
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }
}
