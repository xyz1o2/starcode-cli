use super::*;

pub(super) const PLUGIN_RUNTIME_MANIFEST_CANDIDATES: &[&str] = &[
    ".star-plugin/plugin.json",
    ".claw-plugin/plugin.json",
    ".codex-plugin/plugin.json",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginEntry {
    pub name: String,
    pub source: String,
    pub install_type: String,
    pub installed_at: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugins: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRuntimeManifest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub hooks: HashMap<String, Vec<PluginHookSpec>>,
    #[serde(default)]
    pub commands: Vec<PluginRuntimeCommand>,
    #[serde(default)]
    pub lifecycle: PluginRuntimeLifecycle,
    #[serde(default)]
    pub tools: Vec<PluginRuntimeTool>,
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: HashMap<String, PluginMcpServerConfig>,
}

impl PluginRuntimeManifest {
    pub fn enabled_hook_count(&self) -> usize {
        self.hooks
            .values()
            .flat_map(|hooks| hooks.iter())
            .filter(|hook| hook.is_enabled())
            .count()
    }

    pub fn enabled_command_count(&self) -> usize {
        self.commands
            .iter()
            .filter(|command| command.is_enabled())
            .count()
    }

    pub fn enabled_tool_count(&self) -> usize {
        self.tools.iter().filter(|tool| tool.is_enabled()).count()
    }

    pub fn enabled_lifecycle_count(&self) -> usize {
        self.lifecycle.enabled_command_count()
    }

    pub fn enabled_mcp_server_count(&self) -> usize {
        self.mcp_servers
            .iter()
            .filter(|(_, config)| config.disabled != Some(true))
            .count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PluginHookSpec {
    Command(String),
    Detailed(PluginHookCommand),
}

impl PluginHookSpec {
    pub(super) fn command_spec(&self) -> PluginHookCommand {
        match self {
            Self::Command(command) => PluginHookCommand {
                command: command.clone(),
                ..PluginHookCommand::default()
            },
            Self::Detailed(spec) => spec.clone(),
        }
    }

    pub(super) fn is_enabled(&self) -> bool {
        self.command_spec().enabled != Some(false)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginHookCommand {
    pub command: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub blocking: Option<bool>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRuntimeCommand {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl PluginRuntimeCommand {
    pub(super) fn is_enabled(&self) -> bool {
        self.enabled != Some(false)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginToolPermission {
    #[default]
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl PluginToolPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginRuntimeTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema", default = "default_plugin_tool_input_schema")]
    pub input_schema: Value,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(rename = "requiredPermission", default)]
    pub required_permission: PluginToolPermission,
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl PluginRuntimeTool {
    pub(super) fn is_enabled(&self) -> bool {
        self.enabled != Some(false)
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub entry: PluginEntry,
    pub root: PathBuf,
    pub root_exists: bool,
    pub manifest_path: Option<PathBuf>,
    pub runtime_manifest: Option<PluginRuntimeManifest>,
    pub runtime_error: Option<String>,
    pub runtime_warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PluginHookRegistration {
    pub name: String,
    pub event: String,
    pub command: String,
    pub timeout_secs: u64,
    pub blocking: bool,
    pub source: String,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ResolvedPluginCommand {
    pub name: String,
    pub description: String,
    pub command: String,
    pub timeout_secs: u64,
    pub source: String,
    pub plugin_name: String,
    pub working_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ResolvedPluginTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub command: String,
    pub args: Vec<String>,
    pub required_permission: PluginToolPermission,
    pub source: String,
    pub plugin_name: String,
    pub working_dir: PathBuf,
    pub project_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PluginCommandExecution {
    pub command_name: String,
    pub plugin_name: String,
    pub source: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct PluginEnabledUpdate {
    pub entry: PluginEntry,
    pub previous_enabled: bool,
    pub changed: bool,
}
