//! Sandbox configuration types

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Sandbox execution mode
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// No sandboxing - direct execution
    #[default]
    None,
    /// Linux: bubblewrap namespace isolation
    #[cfg(target_os = "linux")]
    Bubblewrap,
    /// macOS: Seatbelt (sandbox-exec) process isolation
    #[cfg(target_os = "macos")]
    Seatbelt,
    /// Windows: Docker container isolation
    #[cfg(target_os = "windows")]
    Docker,
    /// High security: SmolVM microVM isolation (Linux/macOS)
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    SmolVM,
    /// Enterprise: OpenSandbox server integration
    OpenSandbox,
}

/// Filesystem access rule
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemRule {
    /// Read-only bind mount
    ReadOnly {
        path: PathBuf,
        target: Option<PathBuf>,
    },
    /// Read-write bind mount
    ReadWrite {
        path: PathBuf,
        target: Option<PathBuf>,
    },
    /// Deny access to path
    Deny { path: PathBuf },
    /// Temporary filesystem (tmpfs)
    Tmpfs {
        mount_point: PathBuf,
        size_mb: Option<u32>,
    },
}

/// Network access rule
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkRule {
    /// Allow specific domain (supports wildcards like *.example.com)
    AllowDomain(String),
    /// Deny specific domain
    DenyDomain(String),
    /// Allow specific host:port
    AllowHost { host: String, port: u16 },
    /// Deny specific host:port
    DenyHost { host: String, port: u16 },
    /// Allow localhost connections
    AllowLocalhost,
    /// Deny all network access
    DenyAll,
    /// Default action for unmatched connections
    DefaultAction { allow: bool },
}

/// Sandbox configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Sandbox execution mode
    pub mode: SandboxMode,

    /// Working directory inside sandbox
    pub workdir: Option<PathBuf>,

    /// Filesystem rules
    pub filesystem: Vec<FilesystemRule>,

    /// Network rules
    pub network: NetworkConfig,

    /// Environment variables to pass through
    pub env_passthrough: Vec<String>,

    /// Environment variables to set
    pub env_set: std::collections::HashMap<String, String>,

    /// Resource limits
    pub resources: Option<ResourceLimits>,

    /// OpenSandbox specific configuration
    pub opensandbox: Option<OpenSandboxConfig>,

    /// Docker specific configuration
    pub docker: Option<DockerConfig>,

    /// SmolVM specific configuration
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub smolvm: Option<crate::core::sandbox::smolvm::SmolVMConfig>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::default(),
            workdir: None,
            filesystem: Vec::new(),
            network: NetworkConfig::default(),
            env_passthrough: vec![
                "PATH".to_string(),
                "HOME".to_string(),
                "USER".to_string(),
                "SHELL".to_string(),
            ],
            env_set: std::collections::HashMap::new(),
            resources: None,
            opensandbox: None,
            docker: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            smolvm: None,
        }
    }
}

impl SandboxConfig {
    /// Create a new sandbox config with default settings
    pub fn new(mode: SandboxMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    /// Create a minimal sandbox config for project isolation
    pub fn minimal(project_dir: PathBuf) -> Self {
        Self {
            mode: SandboxMode::default(),
            workdir: Some(project_dir.clone()),
            filesystem: vec![
                FilesystemRule::ReadWrite {
                    path: project_dir,
                    target: None,
                },
                FilesystemRule::Deny {
                    path: PathBuf::from("*.env"),
                },
                FilesystemRule::Deny {
                    path: PathBuf::from("~/.ssh"),
                },
                FilesystemRule::Deny {
                    path: PathBuf::from("~/.aws"),
                },
            ],
            network: NetworkConfig {
                default_action: false, // deny by default
                allowed_domains: vec![
                    "api.anthropic.com".to_string(),
                    "api.openai.com".to_string(),
                    "github.com".to_string(),
                    "*.github.com".to_string(),
                    "pypi.org".to_string(),
                    "*.pypi.org".to_string(),
                    "registry.npmjs.org".to_string(),
                ],
                denied_domains: Vec::new(),
                allow_localhost: true,
            },
            ..Default::default()
        }
    }

    /// Add a read-only filesystem rule
    pub fn with_read_only(mut self, path: PathBuf) -> Self {
        self.filesystem
            .push(FilesystemRule::ReadOnly { path, target: None });
        self
    }

    /// Add a read-write filesystem rule
    pub fn with_read_write(mut self, path: PathBuf) -> Self {
        self.filesystem
            .push(FilesystemRule::ReadWrite { path, target: None });
        self
    }

    /// Add a deny filesystem rule
    pub fn with_deny(mut self, path: PathBuf) -> Self {
        self.filesystem.push(FilesystemRule::Deny { path });
        self
    }

    /// Add an allowed domain
    pub fn with_allowed_domain(mut self, domain: String) -> Self {
        self.network.allowed_domains.push(domain);
        self
    }
}

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Default action for unmatched connections (true = allow, false = deny)
    pub default_action: bool,

    /// Allowed domains
    pub allowed_domains: Vec<String>,

    /// Denied domains
    pub denied_domains: Vec<String>,

    /// Allow localhost connections
    pub allow_localhost: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            default_action: false, // deny by default
            allowed_domains: Vec::new(),
            denied_domains: Vec::new(),
            allow_localhost: true,
        }
    }
}

/// Resource limits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// CPU limit (percentage, 0-100)
    pub cpu_percent: Option<u32>,

    /// Memory limit in MB
    pub memory_mb: Option<u32>,

    /// Maximum number of processes
    pub max_pids: Option<u32>,

    /// Execution timeout in seconds
    pub timeout_secs: Option<u64>,
}

/// OpenSandbox server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenSandboxConfig {
    /// Server URL
    pub server_url: String,

    /// API key for authentication
    pub api_key: Option<String>,

    /// Container image to use
    pub image: String,

    /// Sandbox TTL in seconds
    pub ttl_secs: Option<u64>,
}

/// Docker container configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    /// Docker image to use
    pub image: String,

    /// Container name prefix
    pub name_prefix: Option<String>,

    /// Remove container after execution
    pub auto_remove: bool,

    /// Network mode (bridge, host, none)
    pub network_mode: String,

    /// Additional capabilities
    pub capabilities: Vec<String>,

    /// Drop capabilities
    pub drop_capabilities: Vec<String>,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            image: "ubuntu:22.04".to_string(),
            name_prefix: Some("starcode-sandbox-".to_string()),
            auto_remove: true,
            network_mode: "none".to_string(),
            capabilities: Vec::new(),
            drop_capabilities: vec!["ALL".to_string()],
        }
    }
}
