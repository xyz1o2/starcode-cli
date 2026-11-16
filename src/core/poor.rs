//! `/poor` 省电模式的设置层。
//!
//! ## 它到底管什么
//!
//! 只有一件事：**本地的主动建议**（`core::proactive::ProactiveSuggestions`）。
//!
//! 它**不**管记忆抽取，也**不**减少后台模型调用 —— 本仓库里没有自动记忆
//! 提取的生产消费者（`core::memory::extract::MemoryExtractor` 与
//! `core::budget_mode::BudgetModeManager` 都没有构造点），也没有被 poor
//! 门控的后台模型任务。所以 `/poor` 的文案不得声称"跳过记忆提取"或"减少
//! 后台模型调用" —— 那是文案在撒谎，界面上的承诺兑现不了。
//!
//! ## 用户偏好 vs 临时覆盖
//!
//! poor 是**临时**覆盖：打开时主动建议一律关闭，但用户自己在 `/proactive`
//! 里做的选择要**恢复**。所以磁盘上存两个独立的值：
//!
//! - `poor_mode` —— poor 是否开启；
//! - `proactive_suggestions` —— 用户**本来**想要的主动建议开关。
//!
//! 有效值 = `!poor_mode && proactive_suggestions`。用户偏好被 poor 压住时
//! 依然照记 —— 关闭 poor 后它才回来。
//!
//! ## 优先级
//!
//! 项目层（`.star/settings.json`）覆盖用户层（`user-settings.json`）——
//! 与 `SettingsManager::apply_env_block` 的合并顺序一致。
//!
//! 交互命令**只写用户层**：项目层是共享配置，命令不该擅自改别人的仓库。

use crate::core::config::settings_manager::{ProjectSettings, SettingsManager, UserSettings};

/// `/poor` 与主动建议的启动状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoorStartup {
    /// poor 模式是否开启。
    pub poor_mode: bool,
    /// 用户对主动建议的偏好（poor 关闭后按它恢复）。
    pub proactive_preference: bool,
}

impl Default for PoorStartup {
    fn default() -> Self {
        Self {
            poor_mode: false,
            proactive_preference: true,
        }
    }
}

impl PoorStartup {
    /// 主动建议的**有效**开关。
    pub fn proactive_effective(&self) -> bool {
        !self.poor_mode && self.proactive_preference
    }
}

/// 解析启动状态：项目层覆盖用户层，缺省值取默认（poor 关、主动建议开）。
pub fn resolve(user: &UserSettings, project: &ProjectSettings) -> PoorStartup {
    PoorStartup {
        poor_mode: project.poor_mode.or(user.poor_mode).unwrap_or(false),
        proactive_preference: project
            .proactive_suggestions
            .or(user.proactive_suggestions)
            .unwrap_or(true),
    }
}

/// 仅将 Some 指定的偏好写入用户层；None 保留磁盘原值。
///
/// 只写用户层：项目层的 `.star/settings.json` 是共享配置，交互命令不该
/// 擅自改它 —— 项目层的值只在启动时作为高优先级来源读取。
pub async fn save_preferences(
    manager: &SettingsManager,
    poor_mode: Option<bool>,
    proactive_preference: Option<bool>,
) -> Result<(), String> {
    let mut settings = manager
        .load_user_settings()
        .await
        .map_err(|e| e.to_string())?;
    // 只更新命令明确修改的项，避免把项目层合并值写回全局用户偏好。
    if let Some(value) = poor_mode {
        settings.poor_mode = Some(value);
    }
    if let Some(value) = proactive_preference {
        settings.proactive_suggestions = Some(value);
    }
    manager
        .save_user_settings(&settings)
        .await
        .map_err(|e| e.to_string())
}

/// 项目层是否会**覆盖**用户刚做的选择。
///
/// 项目层优先级更高（见 [`resolve`]），所以用户在这里写下的用户层偏好
/// 可能根本轮不到生效 —— 下次启动又回到项目层定的值。用户必须知道这件事，
/// 否则 `/poor off` 说「已关闭」，重启后它又自己回来了。
///
/// 两个字段**各自独立**判断：项目层只钉了 `poorMode` 时，它压住的只有
/// poor 那一项，主动建议的偏好照旧由用户层决定。
///
/// 返回 `Some(键名)` 表示项目层确实压住了用户层。
pub fn project_override(
    project: &ProjectSettings,
    poor_mode: bool,
    preference: bool,
) -> Option<&'static str> {
    if project.poor_mode.is_some_and(|value| value != poor_mode) {
        Some("poorMode")
    } else if project
        .proactive_suggestions
        .is_some_and(|value| value != preference)
    {
        Some("proactiveSuggestions")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 项目层覆盖用户层 —— 与 `apply_env_block` 的合并顺序一致。
    #[test]
    fn the_project_layer_wins_over_the_user_layer() {
        let user = UserSettings {
            poor_mode: Some(false),
            proactive_suggestions: Some(true),
            ..Default::default()
        };
        let project = ProjectSettings {
            poor_mode: Some(true),
            proactive_suggestions: Some(false),
            ..Default::default()
        };

        let resolved = resolve(&user, &project);
        assert!(resolved.poor_mode);
        assert!(!resolved.proactive_preference);
        assert!(!resolved.proactive_effective());
    }

    /// 用户层单独说了算（项目层没写）。
    #[test]
    fn the_user_layer_applies_when_the_project_layer_is_silent() {
        let user = UserSettings {
            poor_mode: Some(true),
            proactive_suggestions: Some(false),
            ..Default::default()
        };

        let resolved = resolve(&user, &ProjectSettings::default());
        assert!(resolved.poor_mode);
        assert!(!resolved.proactive_preference);
    }

    /// 什么都没写 = 默认：poor 关，主动建议开。
    #[test]
    fn nothing_configured_means_poor_off_and_suggestions_on() {
        let resolved = resolve(&UserSettings::default(), &ProjectSettings::default());
        assert!(!resolved.poor_mode);
        assert!(resolved.proactive_preference);
        assert!(resolved.proactive_effective());
    }

    /// 项目层压住用户层时必须能说出来 —— 否则用户改了偏好、重启发现
    /// 又变回去，以为命令坏了。
    #[test]
    fn a_project_layer_that_disagrees_is_reported() {
        let project = ProjectSettings {
            poor_mode: Some(true),
            proactive_suggestions: None,
            ..Default::default()
        };

        // 用户想关 poor，项目层开着 —— 压住了。
        assert_eq!(project_override(&project, false, true), Some("poorMode"));

        // 用户与项目层一致 —— 没压住。
        assert_eq!(project_override(&project, true, true), None);
    }

    /// 项目层没写这一项时，用户层的选择说了算 —— 不该报"被覆盖"。
    #[test]
    fn a_silent_project_layer_does_not_override_anything() {
        let project = ProjectSettings::default();
        assert_eq!(project_override(&project, false, true), None);
        assert_eq!(project_override(&project, true, false), None);
    }

    /// poor 打开时有效值恒为关；关闭后按用户偏好回来。
    #[test]
    fn poor_overrides_the_preference_only_while_it_is_on() {
        let resolved = PoorStartup {
            poor_mode: true,
            proactive_preference: true,
        };
        assert!(!resolved.proactive_effective());

        let restored = PoorStartup {
            poor_mode: false,
            proactive_preference: true,
        };
        assert!(restored.proactive_effective());
    }
}
