use crate::core::permission_rules::{PermissionAction, PermissionRuleEngine};
use crate::core::policy::settings_rules::{RuleVerdict, SettingsPermissions};
use crate::core::policy::types::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub args: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub decision: PolicyDecision,
    pub rule: Option<PolicyRule>,
}

pub struct PolicyEngine {
    rules: Vec<PolicyRule>,
    checkers: Vec<SafetyCheckerRule>,
    hook_checkers: Vec<HookCheckerRule>,
    default_decision: PolicyDecision,
    non_interactive: bool,
    allow_hooks: bool,
    approval_mode: ApprovalMode,
    session_allowances: std::collections::HashSet<String>, // tool_name
    permission_rule_engine: PermissionRuleEngine,
    /// settings.json `permissions` 段里的 allow/ask/deny 规则
    settings_permissions: SettingsPermissions,
}

impl PolicyEngine {
    pub fn new(config: PolicyEngineConfig) -> Self {
        let mut rules = config.rules.unwrap_or_default();
        let mut checkers = config.checkers.unwrap_or_default();
        let mut hook_checkers = config.hook_checkers.unwrap_or_default();

        rules.sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
        checkers.sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
        hook_checkers.sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));

        Self {
            rules,
            checkers,
            hook_checkers,
            default_decision: config.default_decision.unwrap_or(PolicyDecision::AskUser),
            non_interactive: config.non_interactive.unwrap_or(false),
            allow_hooks: config.allow_hooks.unwrap_or(true),
            approval_mode: config.approval_mode.unwrap_or(ApprovalMode::Default),
            session_allowances: std::collections::HashSet::new(),
            permission_rule_engine: PermissionRuleEngine::new(),
            settings_permissions: SettingsPermissions::default(),
        }
    }

    /// 构造引擎并把磁盘上的权限配置一并装载进来。
    ///
    /// `new` 本身保持纯函数（单测不该读到跑测试那台机器的 `~/.star`），所以装载走这个
    /// 显式入口 —— 每个真正参与运行时的构造点都必须用它，否则用户配的规则就是废纸。
    pub fn with_project_rules(config: PolicyEngineConfig, cwd: &std::path::Path) -> Self {
        let mut engine = Self::new(config);
        engine.load_project_permissions(cwd);
        engine
    }

    /// 装载 `.star/permissions.json`（旧格式）+ settings.json 的 `permissions` 段（新格式）。
    pub fn load_project_permissions(&mut self, cwd: &std::path::Path) {
        let home = dirs::home_dir().unwrap_or_else(|| cwd.to_path_buf());
        self.load_permission_rules(&cwd.to_path_buf(), &home);
        self.settings_permissions = SettingsPermissions::from_project(cwd);
        let legacy = self.permission_rule_engine.get_rules().len();
        if legacy > 0 || !self.settings_permissions.is_empty() {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Policy] Loaded permission rules: settings allow={} ask={} deny={}, legacy={}",
                self.settings_permissions.allow.len(),
                self.settings_permissions.ask.len(),
                self.settings_permissions.deny.len(),
                legacy
            ));
        }
    }

    pub fn settings_permissions(&self) -> &SettingsPermissions {
        &self.settings_permissions
    }

    pub fn set_settings_permissions(&mut self, permissions: SettingsPermissions) {
        self.settings_permissions = permissions;
    }

    pub fn load_permission_rules(
        &mut self,
        project_dir: &std::path::PathBuf,
        home_dir: &std::path::PathBuf,
    ) {
        self.permission_rule_engine
            .load_rules_from_files(project_dir, home_dir);
    }

    pub fn get_permission_rule_engine(&self) -> &PermissionRuleEngine {
        &self.permission_rule_engine
    }

    pub fn get_permission_rule_engine_mut(&mut self) -> &mut PermissionRuleEngine {
        &mut self.permission_rule_engine
    }

    pub fn allow_tool_for_session(&mut self, tool_name: String) {
        self.session_allowances.insert(tool_name);
    }

    pub fn set_approval_mode(&mut self, mode: ApprovalMode) {
        self.approval_mode = mode;
    }

    pub fn get_approval_mode(&self) -> ApprovalMode {
        self.approval_mode.clone()
    }

    /// 判定顺序（对标 Claude Code）：
    ///
    /// 1. `deny` 规则 —— 连 yolo 和"本会话总是允许"都压不住，用户写 deny 就是要绝对拦住；
    /// 2. 会话内的"总是允许"；
    /// 3. 审批模式（yolo 全放 / plan 只放只读）；
    /// 4. `allow` 规则 —— 命中即免确认，这是 allow 存在的唯一意义；
    /// 5. `ask` 规则 —— 命中则强制弹确认，哪怕默认是放行；
    /// 6. 旧的 `PolicyRule` 列表，最后落到 `default_decision`。
    pub async fn check(&self, tool_call: &FunctionCall, server_name: Option<&str>) -> CheckResult {
        let args = tool_call.args.clone().unwrap_or(serde_json::Value::Null);
        let settings_verdict = self.settings_permissions.evaluate(&tool_call.name, &args);
        let legacy_verdict = self.legacy_verdict(&tool_call.name, &args);

        // 1. deny 优先于一切
        for (verdict, reason) in [&settings_verdict, &legacy_verdict].into_iter().flatten() {
            if *verdict == RuleVerdict::Deny {
                return CheckResult {
                    decision: PolicyDecision::DenyWithReason(reason.clone()),
                    rule: None,
                };
            }
        }

        // 2. 会话内已经点过"总是允许"
        if self.session_allowances.contains(&tool_call.name) {
            return CheckResult {
                decision: PolicyDecision::Allow,
                rule: None,
            };
        }

        // 3. 审批模式
        match self.approval_mode {
            ApprovalMode::Yolo => {
                return CheckResult {
                    decision: PolicyDecision::Allow,
                    rule: None,
                };
            }
            ApprovalMode::Plan => {
                // Plan 模式只放只读工具和 exit_plan_mode。重新进入 plan 模式没意义，
                // 还会把 agent 困在 enter/exit 循环里，准备好了就用 exit_plan_mode。
                let is_safe = crate::types::is_safe_query_tool(&tool_call.name);
                let is_exit = tool_call.name == "exit_plan_mode";
                return CheckResult {
                    decision: if is_safe || is_exit {
                        PolicyDecision::Allow
                    } else {
                        PolicyDecision::Deny
                    },
                    rule: None,
                };
            }
            ApprovalMode::Default => {}
        }

        // 4./5. allow 命中就免确认；ask 命中就强制确认
        for (verdict, _) in [&settings_verdict, &legacy_verdict].into_iter().flatten() {
            match verdict {
                RuleVerdict::Allow => {
                    return CheckResult {
                        decision: PolicyDecision::Allow,
                        rule: None,
                    };
                }
                RuleVerdict::Ask => {
                    return CheckResult {
                        decision: self.apply_non_interactive_mode(PolicyDecision::AskUser),
                        rule: None,
                    };
                }
                RuleVerdict::Deny => {}
            }
        }

        let stringified_args = if tool_call.args.is_some() {
            serde_json::to_string(&tool_call.args).ok()
        } else {
            None
        };

        let mut matched_rule: Option<PolicyRule> = None;
        let mut decision: Option<PolicyDecision> = None;

        for rule in &self.rules {
            if self.rule_matches(rule, tool_call, stringified_args.as_deref(), server_name) {
                decision = Some(self.apply_non_interactive_mode(rule.decision.clone()));
                matched_rule = Some(rule.clone());
                break;
            }
        }

        if decision.is_none() {
            decision = Some(self.apply_non_interactive_mode(self.default_decision.clone()));
        }

        CheckResult {
            decision: decision.unwrap(),
            rule: matched_rule,
        }
    }

    /// 旧 `.star/permissions.json` 引擎的判定，翻成统一的三态。
    ///
    /// 注意 `check_permission` 没命中任何规则时返回的是 `allowed: true`（兜底放行），
    /// 那不算"命中 allow 规则" —— 要是当成命中，整个确认机制就被旁路了。所以这里靠
    /// `rule_id` 区分：只有显式命中的规则才有 id。危险命令拦截是 Deny 且没有 id，单独保留。
    fn legacy_verdict(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<(RuleVerdict, String)> {
        let decision = self
            .permission_rule_engine
            .check_permission(tool_name, args);
        let reason = if decision.reason.is_empty() {
            format!("Tool '{}' denied by permission rules", tool_name)
        } else {
            decision.reason.clone()
        };
        match (&decision.action, decision.rule_id.is_some()) {
            (PermissionAction::Deny, _) => Some((RuleVerdict::Deny, reason)),
            (PermissionAction::Ask(_), _) => Some((RuleVerdict::Ask, reason)),
            (PermissionAction::Allow, true) => Some((RuleVerdict::Allow, reason)),
            (PermissionAction::Allow, false) => None,
        }
    }

    fn rule_matches(
        &self,
        rule: &PolicyRule,
        tool_call: &FunctionCall,
        stringified_args: Option<&str>,
        server_name: Option<&str>,
    ) -> bool {
        if let Some(modes) = &rule.modes {
            if !modes.contains(&self.approval_mode) {
                return false;
            }
        }

        if let Some(tool_name) = &rule.tool_name {
            if tool_name.ends_with("__*") {
                let prefix = &tool_name[..tool_name.len() - 3];
                if let Some(server) = server_name {
                    if server != prefix {
                        return false;
                    }
                }
                if !tool_call.name.starts_with(&format!("{}__", prefix)) {
                    return false;
                }
            } else if tool_call.name != *tool_name {
                return false;
            }
        }

        if let Some(pattern) = &rule.args_pattern {
            if tool_call.args.is_none() {
                return false;
            }
            if let Some(args) = stringified_args {
                let re = regex::Regex::new(pattern);
                if let Ok(regex) = re {
                    if !regex.is_match(args) {
                        return false;
                    }
                }
            }
        }

        true
    }

    fn apply_non_interactive_mode(&self, decision: PolicyDecision) -> PolicyDecision {
        if self.non_interactive && decision == PolicyDecision::AskUser {
            PolicyDecision::Deny
        } else {
            decision
        }
    }

    pub fn add_rule(&mut self, rule: PolicyRule) {
        self.rules.push(rule);
        self.rules
            .sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
    }

    pub fn add_checker(&mut self, checker: SafetyCheckerRule) {
        self.checkers.push(checker);
        self.checkers
            .sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
    }

    pub fn remove_rules_for_tool(&mut self, tool_name: &str) {
        self.rules
            .retain(|rule| rule.tool_name.as_deref() != Some(tool_name));
    }

    pub fn get_rules(&self) -> &[PolicyRule] {
        &self.rules
    }

    pub fn get_checkers(&self) -> &[SafetyCheckerRule] {
        &self.checkers
    }

    pub fn add_hook_checker(&mut self, checker: HookCheckerRule) {
        self.hook_checkers.push(checker);
        self.hook_checkers
            .sort_by(|a, b| b.priority.unwrap_or(0).cmp(&a.priority.unwrap_or(0)));
    }

    pub fn get_hook_checkers(&self) -> &[HookCheckerRule] {
        &self.hook_checkers
    }

    pub async fn check_hook(&self, context: &HookExecutionContext) -> PolicyDecision {
        if !self.allow_hooks {
            return PolicyDecision::Deny;
        }

        if context.trusted_folder == Some(false) && context.hook_source == Some(HookSource::Project)
        {
            return PolicyDecision::Deny;
        }

        PolicyDecision::Allow
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new(PolicyEngineConfig::default())
    }
}

impl Default for PolicyEngineConfig {
    fn default() -> Self {
        Self {
            rules: None,
            checkers: None,
            hook_checkers: None,
            default_decision: None,
            non_interactive: None,
            allow_hooks: None,
            approval_mode: None,
        }
    }
}
