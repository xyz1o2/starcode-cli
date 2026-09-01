use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Agent 模式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMode {
    pub id: String,
    pub name: String,
    pub description: String,
    pub persona: Persona,
    pub settings: ModeSettings,
    pub is_custom: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 人格定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    pub name: String,
    pub avatar: Option<String>,
    pub personality: Vec<String>,
    pub expertise: Vec<String>,
    pub communication_style: String,
    pub tone: String,
}

/// 模式设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeSettings {
    /// 系统提示模板
    pub system_prompt_template: String,
    /// 工具权限
    pub tool_permissions: Vec<String>,
    /// 禁用的工具
    pub disabled_tools: Vec<String>,
    /// 上下文窗口大小
    pub context_window_size: Option<usize>,
    /// 温度参数
    pub temperature: Option<f64>,
    /// 最大输出长度
    pub max_output_length: Option<usize>,
    /// 自定义参数
    pub custom_params: HashMap<String, serde_json::Value>,
    /// 启用的技能
    pub enabled_skills: Vec<String>,
    /// 禁用的技能
    pub disabled_skills: Vec<String>,
}

/// 预定义模式
pub mod presets {
    use super::*;

    pub fn coding_assistant() -> AgentMode {
        AgentMode {
            id: "coding_assistant".to_string(),
            name: "Coding Assistant".to_string(),
            description: "A helpful coding assistant focused on writing and reviewing code".to_string(),
            persona: Persona {
                name: "Coder".to_string(),
                avatar: Some("💻".to_string()),
                personality: vec![
                    "helpful".to_string(),
                    "precise".to_string(),
                    "efficient".to_string(),
                ],
                expertise: vec![
                    "programming".to_string(),
                    "code_review".to_string(),
                    "debugging".to_string(),
                ],
                communication_style: "technical".to_string(),
                tone: "professional".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template: "You are a skilled coding assistant. Focus on writing clean, efficient, and well-documented code.".to_string(),
                tool_permissions: vec!["*".to_string()],
                disabled_tools: vec![],
                context_window_size: Some(100000),
                temperature: Some(0.3),
                max_output_length: Some(4096),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn code_reviewer() -> AgentMode {
        AgentMode {
            id: "code_reviewer".to_string(),
            name: "Code Reviewer".to_string(),
            description: "A thorough code reviewer that identifies issues and suggests improvements".to_string(),
            persona: Persona {
                name: "Reviewer".to_string(),
                avatar: Some("🔍".to_string()),
                personality: vec![
                    "thorough".to_string(),
                    "constructive".to_string(),
                    "detail-oriented".to_string(),
                ],
                expertise: vec![
                    "code_review".to_string(),
                    "best_practices".to_string(),
                    "security".to_string(),
                ],
                communication_style: "analytical".to_string(),
                tone: "constructive".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template: "You are an experienced code reviewer. Analyze code thoroughly and provide constructive feedback.".to_string(),
                tool_permissions: vec!["Read".to_string(), "Grep".to_string(), "Glob".to_string()],
                disabled_tools: vec!["Write".to_string(), "Edit".to_string()],
                context_window_size: Some(50000),
                temperature: Some(0.2),
                max_output_length: Some(8192),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn architect() -> AgentMode {
        AgentMode {
            id: "architect".to_string(),
            name: "Software Architect".to_string(),
            description: "A software architect that helps design system architecture".to_string(),
            persona: Persona {
                name: "Architect".to_string(),
                avatar: Some("🏗️".to_string()),
                personality: vec![
                    "strategic".to_string(),
                    "visionary".to_string(),
                    "systematic".to_string(),
                ],
                expertise: vec![
                    "architecture".to_string(),
                    "system_design".to_string(),
                    "scalability".to_string(),
                ],
                communication_style: "high_level".to_string(),
                tone: "authoritative".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template:
                    "You are a software architect. Help design scalable and maintainable systems."
                        .to_string(),
                tool_permissions: vec!["*".to_string()],
                disabled_tools: vec![],
                context_window_size: Some(150000),
                temperature: Some(0.5),
                max_output_length: Some(16384),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn debug_expert() -> AgentMode {
        AgentMode {
            id: "debug_expert".to_string(),
            name: "Debug Expert".to_string(),
            description: "A debugging expert that helps identify and fix bugs".to_string(),
            persona: Persona {
                name: "Debugger".to_string(),
                avatar: Some("🐛".to_string()),
                personality: vec![
                    "patient".to_string(),
                    "methodical".to_string(),
                    "persistent".to_string(),
                ],
                expertise: vec![
                    "debugging".to_string(),
                    "problem_solving".to_string(),
                    "root_cause_analysis".to_string(),
                ],
                communication_style: "step_by_step".to_string(),
                tone: "supportive".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template:
                    "You are a debugging expert. Help identify and fix bugs systematically."
                        .to_string(),
                tool_permissions: vec!["*".to_string()],
                disabled_tools: vec![],
                context_window_size: Some(100000),
                temperature: Some(0.3),
                max_output_length: Some(8192),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn documentation_writer() -> AgentMode {
        AgentMode {
            id: "documentation_writer".to_string(),
            name: "Documentation Writer".to_string(),
            description: "A documentation writer that creates clear and comprehensive docs"
                .to_string(),
            persona: Persona {
                name: "Documenter".to_string(),
                avatar: Some("📝".to_string()),
                personality: vec![
                    "clear".to_string(),
                    "comprehensive".to_string(),
                    "organized".to_string(),
                ],
                expertise: vec![
                    "technical_writing".to_string(),
                    "documentation".to_string(),
                    "api_docs".to_string(),
                ],
                communication_style: "clear".to_string(),
                tone: "informative".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template:
                    "You are a technical writer. Create clear, comprehensive documentation."
                        .to_string(),
                tool_permissions: vec!["Read".to_string(), "Write".to_string(), "Glob".to_string()],
                disabled_tools: vec!["shell".to_string(), "Bash".to_string()],
                context_window_size: Some(80000),
                temperature: Some(0.4),
                max_output_length: Some(16384),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn security_analyst() -> AgentMode {
        AgentMode {
            id: "security_analyst".to_string(),
            name: "Security Analyst".to_string(),
            description: "A security analyst that identifies vulnerabilities and security issues".to_string(),
            persona: Persona {
                name: "Security".to_string(),
                avatar: Some("🛡️".to_string()),
                personality: vec![
                    "vigilant".to_string(),
                    "paranoid".to_string(),
                    "thorough".to_string(),
                ],
                expertise: vec![
                    "security".to_string(),
                    "vulnerability_analysis".to_string(),
                    "penetration_testing".to_string(),
                ],
                communication_style: "cautious".to_string(),
                tone: "serious".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template: "You are a security analyst. Identify security vulnerabilities and recommend fixes.".to_string(),
                tool_permissions: vec!["*".to_string()],
                disabled_tools: vec![],
                context_window_size: Some(100000),
                temperature: Some(0.2),
                max_output_length: Some(8192),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn performance_optimizer() -> AgentMode {
        AgentMode {
            id: "performance_optimizer".to_string(),
            name: "Performance Optimizer".to_string(),
            description: "A performance expert that optimizes code for speed and efficiency".to_string(),
            persona: Persona {
                name: "Optimizer".to_string(),
                avatar: Some("⚡".to_string()),
                personality: vec![
                    "analytical".to_string(),
                    "data_driven".to_string(),
                    "efficient".to_string(),
                ],
                expertise: vec![
                    "performance".to_string(),
                    "optimization".to_string(),
                    "profiling".to_string(),
                ],
                communication_style: "data_driven".to_string(),
                tone: "precise".to_string(),
            },
            settings: ModeSettings {
                system_prompt_template: "You are a performance optimization expert. Analyze and optimize code for maximum efficiency.".to_string(),
                tool_permissions: vec!["*".to_string()],
                disabled_tools: vec![],
                context_window_size: Some(100000),
                temperature: Some(0.3),
                max_output_length: Some(8192),
                custom_params: HashMap::new(),
                enabled_skills: vec![],
                disabled_skills: vec![],
            },
            is_custom: false,
            created_at: None,
        }
    }

    pub fn get_all_presets() -> Vec<AgentMode> {
        vec![
            coding_assistant(),
            code_reviewer(),
            architect(),
            debug_expert(),
            documentation_writer(),
            security_analyst(),
            performance_optimizer(),
        ]
    }
}

/// 模式管理器
pub struct ModeManager {
    modes: Arc<RwLock<HashMap<String, AgentMode>>>,
    current_mode: Arc<RwLock<Option<String>>>,
    config_path: Option<PathBuf>,
}

impl ModeManager {
    pub fn new() -> Self {
        let mut modes = HashMap::new();

        // 加载预设模式
        for mode in presets::get_all_presets() {
            modes.insert(mode.id.clone(), mode);
        }

        Self {
            modes: Arc::new(RwLock::new(modes)),
            current_mode: Arc::new(RwLock::new(None)),
            config_path: None,
        }
    }

    /// 创建带配置路径的管理器
    pub fn with_config_path(config_path: PathBuf) -> Self {
        let mut manager = Self::new();
        manager.config_path = Some(config_path.clone());

        // 尝试加载自定义模式
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(custom_modes) = serde_json::from_str::<Vec<AgentMode>>(&content) {
                    let mut modes = manager.modes.blocking_write();
                    for mode in custom_modes {
                        modes.insert(mode.id.clone(), mode);
                    }
                }
            }
        }

        manager
    }

    /// 保存自定义模式
    async fn save_custom_modes(&self) -> Result<(), String> {
        if let Some(path) = &self.config_path {
            let modes = self.modes.read().await;
            let custom_modes: Vec<&AgentMode> = modes.values().filter(|m| m.is_custom).collect();

            let content = serde_json::to_string_pretty(&custom_modes)
                .map_err(|e| format!("Failed to serialize modes: {}", e))?;

            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }

            std::fs::write(path, content).map_err(|e| format!("Failed to write config: {}", e))?;
        }
        Ok(())
    }

    /// 注册新模式
    pub async fn register_mode(&self, mut mode: AgentMode) -> Result<(), String> {
        mode.is_custom = true;
        mode.created_at = Some(chrono::Utc::now());

        let mut modes = self.modes.write().await;
        modes.insert(mode.id.clone(), mode);

        self.save_custom_modes().await?;
        Ok(())
    }

    /// 获取所有模式
    pub async fn get_all_modes(&self) -> Vec<AgentMode> {
        let modes = self.modes.read().await;
        modes.values().cloned().collect()
    }

    /// 获取单个模式
    pub async fn get_mode(&self, mode_id: &str) -> Option<AgentMode> {
        let modes = self.modes.read().await;
        modes.get(mode_id).cloned()
    }

    /// 设置当前模式
    pub async fn set_current_mode(&self, mode_id: &str) -> Result<(), String> {
        let modes = self.modes.read().await;
        if !modes.contains_key(mode_id) {
            return Err(format!("Mode '{}' not found", mode_id));
        }

        let mut current = self.current_mode.write().await;
        *current = Some(mode_id.to_string());

        tracing::info!("Switched to mode: {}", mode_id);
        Ok(())
    }

    /// 获取当前模式
    pub async fn get_current_mode(&self) -> Option<AgentMode> {
        let current = self.current_mode.read().await;
        if let Some(mode_id) = &*current {
            let modes = self.modes.read().await;
            modes.get(mode_id).cloned()
        } else {
            None
        }
    }

    /// 获取当前模式ID
    pub async fn get_current_mode_id(&self) -> Option<String> {
        let current = self.current_mode.read().await;
        current.clone()
    }

    /// 清除当前模式
    pub async fn clear_current_mode(&self) {
        let mut current = self.current_mode.write().await;
        *current = None;
    }

    /// 删除模式
    pub async fn delete_mode(&self, mode_id: &str) -> Result<(), String> {
        let mut modes = self.modes.write().await;
        let mode = modes
            .get(mode_id)
            .ok_or_else(|| format!("Mode '{}' not found", mode_id))?;

        if !mode.is_custom {
            return Err(format!("Cannot delete preset mode '{}'", mode_id));
        }

        // 检查是否是当前模式
        let current = self.current_mode.read().await;
        if current.as_deref() == Some(mode_id) {
            return Err(format!("Cannot delete currently active mode '{}'", mode_id));
        }

        modes.remove(mode_id);
        drop(modes);

        self.save_custom_modes().await?;
        Ok(())
    }

    /// 获取模式的系统提示
    pub async fn get_system_prompt(&self, mode_id: &str) -> Option<String> {
        let modes = self.modes.read().await;
        modes
            .get(mode_id)
            .map(|m| m.settings.system_prompt_template.clone())
    }

    /// 检查工具是否被允许
    pub async fn is_tool_allowed(&self, mode_id: &str, tool_name: &str) -> bool {
        let modes = self.modes.read().await;
        if let Some(mode) = modes.get(mode_id) {
            // 检查是否在禁用列表中
            if mode
                .settings
                .disabled_tools
                .contains(&tool_name.to_string())
            {
                return false;
            }

            // 检查是否在允许列表中
            if mode.settings.tool_permissions.contains(&"*".to_string()) {
                return true;
            }

            mode.settings
                .tool_permissions
                .contains(&tool_name.to_string())
        } else {
            true // 没有模式时允许所有工具
        }
    }

    /// 获取模式的温度参数
    pub async fn get_temperature(&self, mode_id: &str) -> Option<f64> {
        let modes = self.modes.read().await;
        modes.get(mode_id).and_then(|m| m.settings.temperature)
    }

    /// 获取模式的上下文窗口大小
    pub async fn get_context_window_size(&self, mode_id: &str) -> Option<usize> {
        let modes = self.modes.read().await;
        modes
            .get(mode_id)
            .and_then(|m| m.settings.context_window_size)
    }

    /// 获取模式的最大输出长度
    pub async fn get_max_output_length(&self, mode_id: &str) -> Option<usize> {
        let modes = self.modes.read().await;
        modes
            .get(mode_id)
            .and_then(|m| m.settings.max_output_length)
    }
}

impl Clone for ModeManager {
    fn clone(&self) -> Self {
        Self {
            modes: self.modes.clone(),
            current_mode: self.current_mode.clone(),
            config_path: self.config_path.clone(),
        }
    }
}

/// 模式状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeStatus {
    pub current_mode: Option<String>,
    pub available_modes: Vec<String>,
    pub current_mode_details: Option<AgentMode>,
}

impl ModeStatus {
    pub async fn from_manager(manager: &ModeManager) -> Self {
        let modes = manager.get_all_modes().await;
        let current_mode_id = manager.get_current_mode_id().await;
        let current_mode_details = manager.get_current_mode().await;

        Self {
            current_mode: current_mode_id,
            available_modes: modes.iter().map(|m| m.id.clone()).collect(),
            current_mode_details,
        }
    }
}
