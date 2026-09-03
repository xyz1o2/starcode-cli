use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPServerConfig {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub cwd: Option<PathBuf>,
    pub url: Option<String>,
    pub http_url: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub tcp: Option<String>,
    pub transport_type: Option<TransportType>,
    pub timeout: Option<u64>,
    pub trust: Option<bool>,
    pub description: Option<String>,
    pub include_tools: Option<Vec<String>>,
    pub exclude_tools: Option<Vec<String>>,
    pub extension: Option<StarCLIExtension>,
    pub oauth: Option<MCPOAuthConfig>,
    pub auth_provider_type: Option<AuthProviderType>,
    pub target_audience: Option<String>,
    pub target_service_account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransportType {
    Sse,
    Http,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthProviderType {
    DynamicDiscovery,
    GoogleCredentials,
    ServiceAccountImpersonation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub command: SandboxCommand,
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SandboxCommand {
    Docker,
    Podman,
    SandboxExec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilitySettings {
    pub disable_loading_phrases: Option<bool>,
    pub screen_reader: Option<bool>,
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self {
            disable_loading_phrases: None,
            screen_reader: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugCommandSettings {
    pub url_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummarizeToolOutputSettings {
    pub token_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySettings {
    pub enabled: Option<bool>,
    pub target: Option<TelemetryTarget>,
    pub otlp_endpoint: Option<String>,
    pub otlp_protocol: Option<OtlpProtocol>,
    pub log_prompts: Option<bool>,
    pub outfile: Option<String>,
    pub use_collector: Option<bool>,
    pub use_cli_auth: Option<bool>,
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: None,
            target: None,
            otlp_endpoint: None,
            otlp_protocol: None,
            log_prompts: None,
            outfile: None,
            use_collector: None,
            use_cli_auth: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TelemetryTarget {
    Google,
    Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OtlpProtocol {
    Grpc,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputSettings {
    pub format: Option<OutputFormat>,
}

impl Default for OutputSettings {
    fn default() -> Self {
        Self { format: None }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseInvestigatorSettings {
    pub enabled: Option<bool>,
    pub max_num_turns: Option<usize>,
    pub max_time_minutes: Option<u64>,
    pub thinking_budget: Option<i32>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionSetting {
    pub name: String,
    pub description: String,
    pub env_var: String,
    pub sensitive: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ResolvedExtensionSetting {
    pub name: String,
    pub env_var: String,
    pub value: String,
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionAgentSettings {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StarCLIExtension {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInstallMetadata {
    pub source: String,
    pub install_type: ExtensionType,
    pub release_tag: Option<String>,
    pub ref_field: Option<String>,
    pub auto_update: Option<bool>,
    pub allow_pre_release: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExtensionType {
    Git,
    Local,
    Link,
    GithubRelease,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    pub name: String,
    pub handler: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillDefinition {
    pub name: String,
    pub description: String,
    pub handler: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct MCPOAuthConfig {
    pub client_id: String,
    #[zeroize(skip)]
    pub client_secret: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigParameters {
    pub session_id: String,
    /// 是否从已有会话恢复（--resume / --continue）。仅此时 Agent 才加载
    /// 持久化的会话消息；新启动一律从空上下文开始（对标 Claude Code）。
    pub resume_session: bool,
    pub sandbox: Option<SandboxConfig>,
    pub target_dir: PathBuf,
    pub debug_mode: bool,
    pub question: Option<String>,
    pub core_tools: Option<Vec<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub exclude_tools: Option<Vec<String>>,
    pub tool_discovery_command: Option<String>,
    pub tool_call_command: Option<String>,
    pub mcp_server_command: Option<String>,
    pub mcp_servers: Option<HashMap<String, MCPServerConfig>>,
    pub user_memory: Option<String>,
    pub star_md_file_count: Option<usize>,
    pub star_md_file_paths: Option<Vec<String>>,
    pub approval_mode: Option<crate::core::policy::ApprovalMode>,
    pub show_memory_usage: Option<bool>,
    pub context_file_name: Option<Vec<String>>,
    pub accessibility: Option<AccessibilitySettings>,
    pub telemetry: Option<TelemetrySettings>,
    pub usage_statistics_enabled: Option<bool>,
    pub file_filtering: Option<FileFiltering>,
    pub checkpointing: Option<bool>,
    pub proxy: Option<String>,
    pub cwd: PathBuf,
    pub bug_command: Option<BugCommandSettings>,
    pub model: String,
    pub max_session_turns: Option<i32>,
    pub list_sessions: Option<bool>,
    pub delete_session: Option<String>,
    pub list_extensions: Option<bool>,
    pub enabled_extensions: Option<Vec<String>>,
    pub enable_extension_reloading: Option<bool>,
    pub allowed_mcp_servers: Option<Vec<String>>,
    pub blocked_mcp_servers: Option<Vec<String>>,
    pub allowed_environment_variables: Option<Vec<String>>,
    pub blocked_environment_variables: Option<Vec<String>>,
    pub enable_environment_variable_redaction: Option<bool>,
    pub no_browser: Option<bool>,
    pub summarize_tool_output: Option<HashMap<String, SummarizeToolOutputSettings>>,
    pub folder_trust: Option<bool>,
    pub ide_mode: Option<bool>,
    pub load_memory_from_include_directories: Option<bool>,
    pub import_format: Option<ImportFormat>,
    pub discovery_max_dirs: Option<usize>,
    pub compression_threshold: Option<f64>,
    pub context_window: Option<usize>,
    pub interactive: Option<bool>,
    pub trusted_folder: Option<bool>,
    pub use_ripgrep: Option<bool>,
    pub enable_interactive_shell: Option<bool>,
    pub skip_next_speaker_check: Option<bool>,
    pub extension_management: Option<bool>,
    pub enable_prompt_completion: Option<bool>,
    pub truncate_tool_output_threshold: Option<usize>,
    pub truncate_tool_output_lines: Option<usize>,
    pub enable_tool_output_truncation: Option<bool>,
    pub use_write_todos: Option<bool>,
    pub output: Option<OutputSettings>,
    pub disable_model_router_for_auth: Option<Vec<AuthType>>,
    pub codebase_investigator_settings: Option<CodebaseInvestigatorSettings>,
    pub introspection_agent_settings: Option<IntrospectionAgentSettings>,
    pub continue_on_failed_api_call: Option<bool>,
    pub retry_fetch_errors: Option<bool>,
    pub enable_shell_output_efficiency: Option<bool>,
    pub shell_tool_inactivity_timeout: Option<u64>,
    pub fake_responses: Option<String>,
    pub record_responses: Option<String>,
    pub pty_info: Option<String>,
    pub disable_yolo_mode: Option<bool>,
    pub enable_hooks: Option<bool>,
    pub hooks: Option<HashMap<String, Vec<HookDefinition>>>,
    pub project_hooks: Option<HashMap<String, Vec<HookDefinition>>>,
    pub preview_features: Option<bool>,
    pub enable_agents: Option<bool>,
    pub skills_support: Option<bool>,
    pub disabled_skills: Option<Vec<String>>,
    pub experimental_jit_context: Option<bool>,
    pub mcp_enabled: Option<bool>,
    pub recursion_depth: Option<usize>,
}

impl Default for ConfigParameters {
    fn default() -> Self {
        Self {
            session_id: "test-session".to_string(),
            resume_session: false,
            sandbox: None,
            target_dir: std::env::current_dir().unwrap_or_default(),
            debug_mode: false,
            question: None,
            core_tools: None,
            allowed_tools: None,
            exclude_tools: None,
            tool_discovery_command: None,
            tool_call_command: None,
            mcp_server_command: None,
            mcp_servers: None,
            user_memory: None,
            star_md_file_count: None,
            star_md_file_paths: None,
            approval_mode: None,
            show_memory_usage: None,
            context_file_name: None,
            accessibility: None,
            telemetry: None,
            usage_statistics_enabled: None,
            file_filtering: None,
            checkpointing: None,
            proxy: None,
            cwd: std::env::current_dir().unwrap_or_default(),
            bug_command: None,
            model: String::new(),
            max_session_turns: None,
            list_sessions: None,
            delete_session: None,
            list_extensions: None,
            enabled_extensions: None,
            enable_extension_reloading: None,
            allowed_mcp_servers: None,
            blocked_mcp_servers: None,
            allowed_environment_variables: None,
            blocked_environment_variables: None,
            enable_environment_variable_redaction: None,
            no_browser: None,
            summarize_tool_output: None,
            folder_trust: None,
            ide_mode: None,
            load_memory_from_include_directories: None,
            import_format: None,
            discovery_max_dirs: None,
            compression_threshold: None,
            context_window: None,
            interactive: None,
            trusted_folder: None,
            use_ripgrep: None,
            enable_interactive_shell: None,
            skip_next_speaker_check: None,
            extension_management: None,
            enable_prompt_completion: None,
            truncate_tool_output_threshold: None,
            truncate_tool_output_lines: None,
            enable_tool_output_truncation: None,
            use_write_todos: None,
            output: None,
            disable_model_router_for_auth: None,
            codebase_investigator_settings: None,
            introspection_agent_settings: None,
            continue_on_failed_api_call: None,
            retry_fetch_errors: None,
            enable_shell_output_efficiency: None,
            shell_tool_inactivity_timeout: None,
            fake_responses: None,
            record_responses: None,
            pty_info: None,
            disable_yolo_mode: None,
            enable_hooks: None,
            hooks: None,
            project_hooks: None,
            preview_features: None,
            enable_agents: None,
            skills_support: None,
            disabled_skills: None,
            experimental_jit_context: None,
            mcp_enabled: None,
            recursion_depth: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AuthType {
    UseStar,
    UseVertexAi,
    UseServiceAccount,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ImportFormat {
    Tree,
    Flat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileFiltering {
    pub respect_git_ignore: Option<bool>,
    pub respect_star_ignore: Option<bool>,
    pub enable_recursive_file_search: Option<bool>,
    pub disable_fuzzy_search: Option<bool>,
}

impl Default for FileFiltering {
    fn default() -> Self {
        Self {
            respect_git_ignore: Some(true),
            respect_star_ignore: Some(true),
            enable_recursive_file_search: Some(true),
            disable_fuzzy_search: Some(false),
        }
    }
}
