use crate::core::config::config_types::*;
use crate::core::config::runtime_services::RuntimeServices;
use crate::core::config::storage::Storage;
use crate::core::config::trusted_folders::TrustedFolders;
use crate::core::policy::ApprovalMode;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub const DEFAULT_TRUNCATE_TOOL_OUTPUT_THRESHOLD: usize = 4_000_000;
pub const DEFAULT_TRUNCATE_TOOL_OUTPUT_LINES: usize = 1000;

#[derive(Clone)]
pub struct Config {
    pub(crate) session_id: String,
    pub(crate) sandbox: Option<SandboxConfig>,
    pub(crate) target_dir: PathBuf,
    pub(crate) debug_mode: bool,
    pub(crate) question: Option<String>,
    pub(crate) core_tools: Option<Vec<String>>,
    pub(crate) allowed_tools: Option<Vec<String>>,
    pub(crate) exclude_tools: Option<Vec<String>>,
    pub(crate) tool_discovery_command: Option<String>,
    pub(crate) tool_call_command: Option<String>,
    pub(crate) mcp_server_command: Option<String>,
    pub(crate) mcp_servers: Option<HashMap<String, MCPServerConfig>>,
    pub(crate) mcp_enabled: bool,
    pub(crate) allowed_mcp_servers: Vec<String>,
    pub(crate) blocked_mcp_servers: Vec<String>,
    pub(crate) allowed_environment_variables: Vec<String>,
    pub(crate) blocked_environment_variables: Vec<String>,
    pub(crate) enable_environment_variable_redaction: bool,
    pub(crate) user_memory: String,
    pub star_md_file_count: usize,
    pub star_md_file_paths: Vec<String>,
    pub(crate) show_memory_usage: bool,
    pub(crate) accessibility: AccessibilitySettings,
    pub(crate) telemetry_settings: TelemetrySettings,
    pub(crate) usage_statistics_enabled: bool,
    pub(crate) checkpointing: bool,
    pub(crate) proxy: Option<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) bug_command: Option<BugCommandSettings>,
    pub(crate) model: String,
    pub(crate) active_model: String,
    pub(crate) max_session_turns: i32,
    pub(crate) list_sessions: bool,
    pub(crate) delete_session: Option<String>,
    pub(crate) list_extensions: bool,
    pub(crate) enabled_extensions: Vec<String>,
    pub(crate) enable_extension_reloading: bool,
    pub(crate) no_browser: bool,
    pub(crate) summarize_tool_output: Option<HashMap<String, SummarizeToolOutputSettings>>,
    pub(crate) folder_trust: bool,
    pub(crate) ide_mode: bool,
    pub(crate) load_memory_from_include_directories: bool,
    pub(crate) import_format: ImportFormat,
    pub(crate) discovery_max_dirs: usize,
    pub(crate) compression_threshold: Option<f64>,
    pub(crate) context_window: usize,
    pub(crate) interactive: bool,
    pub(crate) pty_info: String,
    pub(crate) trusted_folder: Option<bool>,
    pub(crate) use_ripgrep: bool,
    pub(crate) enable_interactive_shell: bool,
    pub(crate) skip_next_speaker_check: bool,
    pub(crate) extension_management: bool,
    pub(crate) enable_prompt_completion: bool,
    pub(crate) truncate_tool_output_threshold: usize,
    pub(crate) truncate_tool_output_lines: usize,
    pub(crate) enable_tool_output_truncation: bool,
    pub(crate) initialized: bool,
    pub(crate) storage: Storage,
    pub(crate) use_write_todos: bool,
    pub(crate) output_settings: OutputSettings,
    pub(crate) codebase_investigator_settings: CodebaseInvestigatorSettings,
    pub(crate) introspection_agent_settings: IntrospectionAgentSettings,
    pub(crate) continue_on_failed_api_call: bool,
    pub(crate) retry_fetch_errors: bool,
    pub(crate) enable_shell_output_efficiency: bool,
    pub(crate) shell_tool_inactivity_timeout: u64,
    pub(crate) fake_responses: Option<String>,
    pub(crate) record_responses: Option<String>,
    pub(crate) disable_yolo_mode: bool,
    pub(crate) pending_include_directories: Vec<String>,
    pub(crate) enable_hooks: bool,
    pub(crate) hooks: Option<HashMap<String, Vec<HookDefinition>>>,
    pub(crate) project_hooks: Option<HashMap<String, Vec<HookDefinition>>>,
    pub(crate) disabled_hooks: Vec<String>,
    pub(crate) preview_features: Option<bool>,
    pub(crate) enable_agents: bool,
    pub(crate) skills_support: bool,
    pub(crate) disabled_skills: Vec<String>,
    pub(crate) experimental_jit_context: bool,
    pub(crate) file_filtering: FileFiltering,
    pub recursion_depth: usize,
    pub policy_engine_approval_mode: ApprovalMode,
    pub runtime_services: Option<Arc<RuntimeServices>>,
    pub trusted_folders_manager: Option<TrustedFolders>,
}

impl Config {
    pub fn new(params: ConfigParameters) -> Self {
        let storage = Storage::new(params.target_dir.clone());
        let trusted_folders_manager = TrustedFolders::new().ok();

        Self {
            trusted_folders_manager,
            session_id: params.session_id,
            sandbox: params.sandbox,
            target_dir: params.target_dir,
            debug_mode: params.debug_mode,
            question: params.question,
            core_tools: params.core_tools,
            allowed_tools: params.allowed_tools,
            exclude_tools: params.exclude_tools,
            tool_discovery_command: params.tool_discovery_command,
            tool_call_command: params.tool_call_command,
            mcp_server_command: params.mcp_server_command,
            mcp_servers: params.mcp_servers,
            mcp_enabled: params.mcp_enabled.unwrap_or(true),
            allowed_mcp_servers: params.allowed_mcp_servers.unwrap_or_default(),
            blocked_mcp_servers: params.blocked_mcp_servers.unwrap_or_default(),
            allowed_environment_variables: params.allowed_environment_variables.unwrap_or_default(),
            blocked_environment_variables: params.blocked_environment_variables.unwrap_or_default(),
            enable_environment_variable_redaction: params
                .enable_environment_variable_redaction
                .unwrap_or(false),
            user_memory: params.user_memory.unwrap_or_default(),
            star_md_file_count: params.star_md_file_count.unwrap_or(0),
            star_md_file_paths: params.star_md_file_paths.unwrap_or_default(),
            show_memory_usage: params.show_memory_usage.unwrap_or(false),
            accessibility: params.accessibility.unwrap_or_default(),
            telemetry_settings: params.telemetry.unwrap_or_default(),
            usage_statistics_enabled: params.usage_statistics_enabled.unwrap_or(true),
            checkpointing: params.checkpointing.unwrap_or(false),
            proxy: params.proxy,
            cwd: params.cwd,
            bug_command: params.bug_command,
            model: params.model.clone(),
            active_model: params.model,
            max_session_turns: params.max_session_turns.unwrap_or(-1),
            list_sessions: params.list_sessions.unwrap_or(false),
            delete_session: params.delete_session,
            list_extensions: params.list_extensions.unwrap_or(false),
            enabled_extensions: params.enabled_extensions.unwrap_or_default(),
            enable_extension_reloading: params.enable_extension_reloading.unwrap_or(false),
            no_browser: params.no_browser.unwrap_or(false),
            summarize_tool_output: params.summarize_tool_output,
            folder_trust: params.folder_trust.unwrap_or(false),
            ide_mode: params.ide_mode.unwrap_or(false),
            load_memory_from_include_directories: params
                .load_memory_from_include_directories
                .unwrap_or(false),
            import_format: params.import_format.unwrap_or(ImportFormat::Tree),
            discovery_max_dirs: params.discovery_max_dirs.unwrap_or(200),
            compression_threshold: params.compression_threshold,
            context_window: params.context_window.unwrap_or(
                std::env::var("STAR_CONTEXT_WINDOW")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(200_000),
            ),
            interactive: params.interactive.unwrap_or(false),
            pty_info: params
                .pty_info
                .unwrap_or_else(|| "child_process".to_string()),
            trusted_folder: params.trusted_folder,
            use_ripgrep: params.use_ripgrep.unwrap_or(true),
            enable_interactive_shell: params.enable_interactive_shell.unwrap_or(false),
            skip_next_speaker_check: params.skip_next_speaker_check.unwrap_or(true),
            extension_management: params.extension_management.unwrap_or(true),
            enable_prompt_completion: params.enable_prompt_completion.unwrap_or(false),
            truncate_tool_output_threshold: params
                .truncate_tool_output_threshold
                .unwrap_or(DEFAULT_TRUNCATE_TOOL_OUTPUT_THRESHOLD),
            truncate_tool_output_lines: params
                .truncate_tool_output_lines
                .unwrap_or(DEFAULT_TRUNCATE_TOOL_OUTPUT_LINES),
            enable_tool_output_truncation: params.enable_tool_output_truncation.unwrap_or(true),
            initialized: false,
            storage,
            use_write_todos: params.use_write_todos.unwrap_or(true),
            output_settings: params.output.unwrap_or_default(),
            codebase_investigator_settings: params.codebase_investigator_settings.unwrap_or_else(
                || CodebaseInvestigatorSettings {
                    enabled: Some(true),
                    max_num_turns: Some(10),
                    max_time_minutes: Some(3),
                    thinking_budget: None,
                    model: None,
                },
            ),
            introspection_agent_settings: params.introspection_agent_settings.unwrap_or_else(
                || IntrospectionAgentSettings {
                    enabled: Some(false),
                },
            ),
            continue_on_failed_api_call: params.continue_on_failed_api_call.unwrap_or(true),
            retry_fetch_errors: params.retry_fetch_errors.unwrap_or(false),
            enable_shell_output_efficiency: params.enable_shell_output_efficiency.unwrap_or(true),
            shell_tool_inactivity_timeout: params.shell_tool_inactivity_timeout.unwrap_or(120)
                * 1000,
            fake_responses: params.fake_responses,
            record_responses: params.record_responses,
            disable_yolo_mode: params.disable_yolo_mode.unwrap_or(false),
            pending_include_directories: vec![],
            enable_hooks: params.enable_hooks.unwrap_or(false),
            hooks: params.hooks,
            project_hooks: params.project_hooks,
            disabled_hooks: vec![],
            preview_features: params.preview_features,
            enable_agents: params.enable_agents.unwrap_or(false),
            skills_support: params.skills_support.unwrap_or(true),
            disabled_skills: params.disabled_skills.unwrap_or_default(),
            experimental_jit_context: params.experimental_jit_context.unwrap_or(false),
            file_filtering: params.file_filtering.unwrap_or_else(|| FileFiltering {
                respect_git_ignore: Some(true),
                respect_star_ignore: Some(true),
                enable_recursive_file_search: Some(true),
                disable_fuzzy_search: Some(false),
            }),
            recursion_depth: params.recursion_depth.unwrap_or(0),
            policy_engine_approval_mode: params.approval_mode.unwrap_or(ApprovalMode::Default),
            runtime_services: None,
        }
    }

    // Getters
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn sandbox(&self) -> Option<&SandboxConfig> {
        self.sandbox.as_ref()
    }
    pub fn target_dir(&self) -> &PathBuf {
        &self.target_dir
    }
    pub fn project_root(&self) -> &PathBuf {
        &self.target_dir
    }
    pub fn debug_mode(&self) -> bool {
        self.debug_mode
    }
    pub fn question(&self) -> Option<&String> {
        self.question.as_ref()
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn active_model(&self) -> &str {
        &self.active_model
    }
    pub fn set_model(&mut self, new_model: String) {
        self.model = new_model.clone();
        self.active_model = new_model;
    }
    pub fn set_active_model(&mut self, model: String) {
        if self.active_model != model {
            self.active_model = model;
        }
    }
    pub fn working_dir(&self) -> &PathBuf {
        &self.cwd
    }
    pub fn storage(&self) -> &Storage {
        &self.storage
    }
    pub fn user_memory(&self) -> &str {
        &self.user_memory
    }
    pub fn set_user_memory(&mut self, memory: String) {
        self.user_memory = memory;
    }
    pub fn star_md_file_count(&self) -> usize {
        self.star_md_file_count
    }
    pub fn star_md_file_paths(&self) -> &[String] {
        &self.star_md_file_paths
    }
    pub fn show_memory_usage(&self) -> bool {
        self.show_memory_usage
    }
    pub fn accessibility(&self) -> &AccessibilitySettings {
        &self.accessibility
    }
    pub fn telemetry_enabled(&self) -> bool {
        self.telemetry_settings.enabled.unwrap_or(false)
    }
    pub fn telemetry_log_prompts_enabled(&self) -> bool {
        self.telemetry_settings.log_prompts.unwrap_or(true)
    }
    pub fn usage_statistics_enabled(&self) -> bool {
        self.usage_statistics_enabled
    }
    pub fn checkpointing_enabled(&self) -> bool {
        self.checkpointing
    }
    pub fn proxy(&self) -> Option<&String> {
        self.proxy.as_ref()
    }
    pub fn bug_command(&self) -> Option<&BugCommandSettings> {
        self.bug_command.as_ref()
    }
    pub fn max_session_turns(&self) -> i32 {
        self.max_session_turns
    }
    pub fn list_sessions(&self) -> bool {
        self.list_sessions
    }
    pub fn delete_session(&self) -> Option<&String> {
        self.delete_session.as_ref()
    }
    pub fn list_extensions(&self) -> bool {
        self.list_extensions
    }
    pub fn enabled_extensions(&self) -> &[String] {
        &self.enabled_extensions
    }
    pub fn enable_extension_reloading(&self) -> bool {
        self.enable_extension_reloading
    }
    pub fn no_browser(&self) -> bool {
        self.no_browser
    }
    pub fn summarize_tool_output(&self) -> Option<&HashMap<String, SummarizeToolOutputSettings>> {
        self.summarize_tool_output.as_ref()
    }
    pub fn folder_trust(&self) -> bool {
        self.folder_trust
    }
    pub fn trusted_folder(&self) -> Option<bool> {
        self.trusted_folder
    }
    pub fn ide_mode(&self) -> bool {
        self.ide_mode
    }
    pub fn set_ide_mode(&mut self, value: bool) {
        self.ide_mode = value;
    }
    pub fn load_memory_from_include_directories(&self) -> bool {
        self.load_memory_from_include_directories
    }
    pub fn import_format(&self) -> &ImportFormat {
        &self.import_format
    }
    pub fn discovery_max_dirs(&self) -> usize {
        self.discovery_max_dirs
    }
    pub fn compression_threshold(&self) -> Option<f64> {
        self.compression_threshold
    }
    pub fn context_window(&self) -> usize {
        // 1. 模型专用上下文窗口：从 API /models 缓存中查（如 Anthropic 的 max_input_tokens）
        if let Some(ctx) =
            crate::agent::model_catalog::get_cached_context_window(self.active_model())
        {
            return ctx as usize;
        }
        // 2. AppConfig / ConfigParameters 中配置的值（含 STAR_CONTEXT_WINDOW env var 回退）
        self.context_window
    }
    pub fn interactive(&self) -> bool {
        self.interactive
    }
    pub fn use_ripgrep(&self) -> bool {
        self.use_ripgrep
    }
    pub fn enable_interactive_shell(&self) -> bool {
        self.enable_interactive_shell
    }
    pub fn skip_next_speaker_check(&self) -> bool {
        self.skip_next_speaker_check
    }
    pub fn continue_on_failed_api_call(&self) -> bool {
        self.continue_on_failed_api_call
    }
    pub fn retry_fetch_errors(&self) -> bool {
        self.retry_fetch_errors
    }
    pub fn enable_shell_output_efficiency(&self) -> bool {
        self.enable_shell_output_efficiency
    }
    pub fn shell_tool_inactivity_timeout(&self) -> u64 {
        self.shell_tool_inactivity_timeout
    }
    pub fn fake_responses(&self) -> Option<&String> {
        self.fake_responses.as_ref()
    }
    pub fn record_responses(&self) -> Option<&String> {
        self.record_responses.as_ref()
    }
    pub fn disable_yolo_mode(&self) -> bool {
        self.disable_yolo_mode
    }
    pub fn enable_hooks(&self) -> bool {
        self.enable_hooks
    }
    pub fn hooks(&self) -> Option<&HashMap<String, Vec<HookDefinition>>> {
        self.hooks.as_ref()
    }
    pub fn project_hooks(&self) -> Option<&HashMap<String, Vec<HookDefinition>>> {
        self.project_hooks.as_ref()
    }
    pub fn disabled_hooks(&self) -> &[String] {
        &self.disabled_hooks
    }
    pub fn preview_features(&self) -> Option<bool> {
        self.preview_features
    }
    pub fn set_preview_features(&mut self, preview_features: bool) {
        if self.preview_features == Some(preview_features) {
            return;
        }
        self.preview_features = Some(preview_features);
        let current_model = self.model.clone();

        if !preview_features && is_preview_model(&current_model) {
            self.set_model(String::new());
        }
    }
    pub fn enable_agents(&self) -> bool {
        self.enable_agents
    }
    pub fn skills_support(&self) -> bool {
        self.skills_support
    }
    pub fn disabled_skills(&self) -> &[String] {
        &self.disabled_skills
    }
    pub fn experimental_jit_context(&self) -> bool {
        self.experimental_jit_context
    }
    pub fn file_filtering(&self) -> &FileFiltering {
        &self.file_filtering
    }
    pub fn approval_mode(&self) -> &ApprovalMode {
        &self.policy_engine_approval_mode
    }
    pub fn set_approval_mode(
        &mut self,
        mode: ApprovalMode,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.is_trusted_folder() && mode != ApprovalMode::Default {
            return Err("Cannot enable privileged approval modes in an untrusted folder.".into());
        }
        self.policy_engine_approval_mode = mode;
        Ok(())
    }
    pub fn is_trusted_folder(&self) -> bool {
        if self.folder_trust {
            self.trusted_folder.unwrap_or(false)
        } else {
            true
        }
    }
    pub fn is_yolo_mode_disabled(&self) -> bool {
        self.disable_yolo_mode || !self.is_trusted_folder()
    }
    pub fn output_format(&self) -> &OutputFormat {
        self.output_settings
            .format
            .as_ref()
            .unwrap_or(&OutputFormat::Text)
    }
    pub fn use_write_todos(&self) -> bool {
        self.use_write_todos
    }
    pub fn enable_tool_output_truncation(&self) -> bool {
        self.enable_tool_output_truncation
    }
    pub fn truncate_tool_output_threshold(&self) -> usize {
        self.truncate_tool_output_threshold
    }
    pub fn truncate_tool_output_lines(&self) -> usize {
        self.truncate_tool_output_lines
    }
    pub fn codebase_investigator_settings(&self) -> &CodebaseInvestigatorSettings {
        &self.codebase_investigator_settings
    }
    pub fn introspection_agent_settings(&self) -> &IntrospectionAgentSettings {
        &self.introspection_agent_settings
    }
    pub fn enable_prompt_completion(&self) -> bool {
        self.enable_prompt_completion
    }
    pub fn is_interactive_shell_enabled(&self) -> bool {
        self.interactive && self.pty_info != "child_process" && self.enable_interactive_shell
    }
    pub fn is_browser_launch_suppressed(&self) -> bool {
        self.no_browser
    }
    pub fn mcp_enabled(&self) -> bool {
        self.mcp_enabled
    }
    pub fn mcp_servers(&self) -> Option<&HashMap<String, MCPServerConfig>> {
        self.mcp_servers.as_ref()
    }
    pub fn allowed_mcp_servers(&self) -> &[String] {
        &self.allowed_mcp_servers
    }
    pub fn blocked_mcp_servers(&self) -> &[String] {
        &self.blocked_mcp_servers
    }
    pub fn allowed_environment_variables(&self) -> &[String] {
        &self.allowed_environment_variables
    }
    pub fn blocked_environment_variables(&self) -> &[String] {
        &self.blocked_environment_variables
    }
    pub fn enable_environment_variable_redaction(&self) -> bool {
        self.enable_environment_variable_redaction
    }
    pub fn core_tools(&self) -> Option<&[String]> {
        self.core_tools.as_deref()
    }
    pub fn allowed_tools(&self) -> Option<&[String]> {
        self.allowed_tools.as_deref()
    }
    pub fn exclude_tools(&self) -> Option<&[String]> {
        self.exclude_tools.as_deref()
    }
    pub fn tool_discovery_command(&self) -> Option<&String> {
        self.tool_discovery_command.as_ref()
    }
    pub fn tool_call_command(&self) -> Option<&String> {
        self.tool_call_command.as_ref()
    }
    pub fn mcp_server_command(&self) -> Option<&String> {
        self.mcp_server_command.as_ref()
    }
    pub fn pending_include_directories(&self) -> &[String] {
        &self.pending_include_directories
    }
    pub fn clear_pending_include_directories(&mut self) {
        self.pending_include_directories.clear();
    }
    pub fn extension_management(&self) -> bool {
        self.extension_management
    }
    pub fn trusted_folders(&self) -> Option<&TrustedFolders> {
        self.trusted_folders_manager.as_ref()
    }
    pub(crate) fn runtime_tool_registry(&self) -> Option<Arc<crate::core::tools::ToolRegistry>> {
        self.runtime_services
            .as_ref()
            .and_then(|services| services.tool_registry())
    }
    pub(crate) fn runtime_message_bus(
        &self,
    ) -> Option<Arc<crate::core::confirmation_bus::MessageBus>> {
        self.runtime_services
            .as_ref()
            .map(|services| services.message_bus())
    }
    pub(crate) fn runtime_mcp_manager(&self) -> Option<Arc<crate::core::mcp::MCPManager>> {
        self.runtime_services
            .as_ref()
            .and_then(|services| services.mcp_manager())
    }
    pub(crate) fn runtime_global_state(&self) -> Option<Arc<crate::core::state::GlobalState>> {
        self.runtime_services
            .as_ref()
            .map(|services| services.global_state())
    }
    pub(crate) fn runtime_notification_queue(
        &self,
    ) -> Option<Arc<tokio::sync::Mutex<crate::agent::subagent::notification::NotificationQueue>>>
    {
        self.runtime_services
            .as_ref()
            .and_then(|services| services.notification_queue())
    }
    pub fn install_runtime_services(&mut self, runtime_services: RuntimeServices) {
        self.runtime_services = Some(Arc::new(runtime_services));
    }
}

fn is_preview_model(model: &str) -> bool {
    model.contains("preview")
}
