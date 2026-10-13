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
    /// 判定是否来自一条**真正命中的规则**（settings 的 allow/ask/deny、旧
    /// `permissions.json` 规则、或 `PolicyEngineConfig.rules`）。
    ///
    /// 存在的理由：`AskUser` 有两个来源，语义完全不同，不能混。
    /// - 显式 `ask` 规则 —— 用户写下"每次都问我"，会话里的"本会话允许"不该把它降级
    /// - 兜底 `default_decision`（默认 `AskUser`）—— 只是没命中任何规则时的保守起见
    ///
    /// 两者混为一谈的后果：没有任何 settings 的普通用户点「本会话允许」后，
    /// 下一次仍然弹框，因为兜底 AskUser 一直被当成"策略点名要求确认"。
    pub explicit_rule: bool,
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

    /// 审批模式**自带**的免确认集合（对标 Claude Code 的模式语义表）。
    ///
    /// 这是模式语义的一部分，不是用户写下的规则，所以不进 `settings_permissions`：
    /// - `default` = Manual —— 只读工具免确认，写与命令才问
    /// - `acceptEdits` —— 再加上文件改动工具与只读 shell 命令
    ///
    /// `Yolo` / `Plan` 在 `check` 里各自 return，不走这里。
    /// deny 与显式 `ask` 的优先级由 `check` 的调用顺序保证（deny 第 1 步、
    /// ask 由 `check` 在调用本方法前查过），本方法只回答"模式承诺放行吗"。
    fn mode_auto_allows(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        match self.approval_mode {
            ApprovalMode::Default => crate::types::is_safe_query_tool(tool_name),
            ApprovalMode::AcceptEdits => {
                crate::types::is_safe_query_tool(tool_name)
                    || crate::types::is_file_edit_tool(tool_name)
                    || crate::types::is_readonly_fs_command(tool_name, args)
            }
            ApprovalMode::Yolo | ApprovalMode::Plan => false,
        }
    }

    /// 判定顺序（对标 Claude Code）：
    ///
    /// 1. `deny` 规则 —— 连 yolo 和"本会话总是允许"都压不住，用户写 deny 就是要绝对拦住；
    /// 2. 会话内的"总是允许"；
    /// 3. 审批模式（yolo 全放 / plan 只放只读 / default 与 acceptEdits 放各自的只读集合）；
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
                    explicit_rule: true,
                };
            }
        }

        // 2. 会话内已经点过"总是允许"
        if self.session_allowances.contains(&tool_call.name) {
            return CheckResult {
                decision: PolicyDecision::Allow,
                rule: None,
                explicit_rule: false,
            };
        }

        // 3. 审批模式
        match self.approval_mode {
            ApprovalMode::Yolo => {
                return CheckResult {
                    decision: PolicyDecision::Allow,
                    rule: None,
                    explicit_rule: false,
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
                    explicit_rule: false,
                };
            }
            // Default / AcceptEdits 不在这里 return：模式内置的放行集合（只读工具、
            // acceptEdits 再加文件编辑与只读命令）要排在显式 `ask` 规则之后生效 ——
            // 用户写 `ask: ["Read"]` 就是"每次都问我"，模式默认值不能把它吃掉。
            ApprovalMode::Default | ApprovalMode::AcceptEdits => {
                let asked_by_rule = [&settings_verdict, &legacy_verdict]
                    .into_iter()
                    .flatten()
                    .any(|(verdict, _)| *verdict == RuleVerdict::Ask);
                if !asked_by_rule && self.mode_auto_allows(&tool_call.name, &args) {
                    return CheckResult {
                        decision: PolicyDecision::Allow,
                        rule: None,
                        explicit_rule: false,
                    };
                }
            }
        }

        // 4./5. allow 命中就免确认；ask 命中就强制确认
        for (verdict, _) in [&settings_verdict, &legacy_verdict].into_iter().flatten() {
            match verdict {
                RuleVerdict::Allow => {
                    return CheckResult {
                        decision: PolicyDecision::Allow,
                        rule: None,
                        explicit_rule: true,
                    };
                }
                RuleVerdict::Ask => {
                    // 显式 ask 规则：用户写下的"每次都问我"。`explicit_rule` 为 true，
                    // 调用方据此让会话里的"本会话允许"失效。
                    return CheckResult {
                        decision: self.apply_non_interactive_mode(PolicyDecision::AskUser),
                        rule: None,
                        explicit_rule: true,
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
            // 兜底：没命中任何规则。默认是 AskUser，但那是"保守起见问一下"，
            // 不是用户写下的规则 —— `explicit_rule` 必须为 false，否则「本会话允许」
            // 会永久失效（这正是本函数原先没有区分、导致的问题）。
            decision = Some(self.apply_non_interactive_mode(self.default_decision.clone()));
        }

        CheckResult {
            decision: decision.unwrap(),
            rule: matched_rule.clone(),
            explicit_rule: matched_rule.is_some(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::policy::settings_rules::{PermissionRuleSpec, SettingsPermissions};

    fn call(name: &str, args: serde_json::Value) -> FunctionCall {
        FunctionCall {
            name: name.to_string(),
            args: Some(args),
        }
    }

    /// 回归：headless（`-p`）下 `AskUser` 必须立刻变成拒绝。
    ///
    /// `-p` 没有 UI 订阅消息总线，`AskUser` 没人应答，调用方会一直等到
    /// 确认超时（默认 600 秒）才失败 —— CI 里表现为每个未授权工具卡十分钟。
    #[tokio::test]
    async fn non_interactive_denies_instead_of_asking() {
        let engine = PolicyEngine::new(PolicyEngineConfig {
            non_interactive: Some(true),
            ..Default::default()
        });

        let decision = engine
            .check(
                &call("Bash", serde_json::json!({"command": "git push"})),
                None,
            )
            .await
            .decision;
        assert_eq!(
            decision,
            PolicyDecision::Deny,
            "headless 下没人能回答确认，必须立刻拒绝"
        );
    }

    /// 反向保证：交互模式保持弹窗，deny 规则依然最优先。
    #[tokio::test]
    async fn interactive_still_asks_and_deny_still_wins() {
        let engine = PolicyEngine::new(PolicyEngineConfig::default());

        assert_eq!(
            engine
                .check(
                    &call("Bash", serde_json::json!({"command": "git push"})),
                    None
                )
                .await
                .decision,
            PolicyDecision::AskUser
        );

        let mut with_deny = PolicyEngine::new(PolicyEngineConfig {
            non_interactive: Some(true),
            ..Default::default()
        });
        let mut perms = SettingsPermissions::default();
        perms
            .deny
            .push(PermissionRuleSpec::parse("Bash(rm:*)").expect("parse"));
        with_deny.set_settings_permissions(perms);
        assert!(matches!(
            with_deny
                .check(
                    &call("Bash", serde_json::json!({"command": "rm -rf build"})),
                    None
                )
                .await
                .decision,
            PolicyDecision::DenyWithReason(_)
        ));
    }

    /// yolo 在 headless 下仍然放行 —— 用户显式要求免确认时不该被 non_interactive 推翻。
    #[tokio::test]
    async fn yolo_still_allows_in_non_interactive_mode() {
        let engine = PolicyEngine::new(PolicyEngineConfig {
            non_interactive: Some(true),
            approval_mode: Some(ApprovalMode::Yolo),
            ..Default::default()
        });
        assert_eq!(
            engine
                .check(
                    &call("Bash", serde_json::json!({"command": "git push"})),
                    None
                )
                .await
                .decision,
            PolicyDecision::Allow
        );
    }

    fn engine_in(mode: ApprovalMode) -> PolicyEngine {
        PolicyEngine::new(PolicyEngineConfig {
            approval_mode: Some(mode),
            ..Default::default()
        })
    }

    /// 回归（P0-1）：`default` = CC 的 Manual —— 只读工具免确认。
    ///
    /// 修复前 `default_decision = AskUser` 会把 Read/Grep/Glob 也弹框，用户跑一个
    /// 任务被打断十几次；headless 更糟：`non_interactive` 把 AskUser 翻成 Deny，
    /// 于是 `-p` 模式下**每个**工具都被拒，agent 连文件都读不了。
    #[tokio::test]
    async fn default_mode_allows_read_only_tools_without_asking() {
        let engine = engine_in(ApprovalMode::Default);
        for tool in ["Read", "Grep", "Glob", "ListDir", "TodoWrite"] {
            let result = engine
                .check(
                    &call(tool, serde_json::json!({"file_path": "src/main.rs"})),
                    None,
                )
                .await;
            assert_eq!(
                result.decision,
                PolicyDecision::Allow,
                "{tool} 是只读工具，default 模式不该弹确认"
            );
            assert!(
                !result.explicit_rule,
                "模式内置放行不是规则命中，`explicit_rule` 必须为 false，否则「本会话允许」会失效"
            );
        }
    }

    /// 同一修复在 headless 下的意义：`-p` 遇只读工具必须是放行而不是拒绝。
    #[tokio::test]
    async fn headless_default_mode_can_read_files() {
        let engine = PolicyEngine::new(PolicyEngineConfig {
            non_interactive: Some(true),
            approval_mode: Some(ApprovalMode::Default),
            ..Default::default()
        });
        assert_eq!(
            engine
                .check(
                    &call("Read", serde_json::json!({"file_path": "Cargo.toml"})),
                    None
                )
                .await
                .decision,
            PolicyDecision::Allow
        );
        // 写操作仍然直接拒绝（headless 没人能回答）
        assert_eq!(
            engine
                .check(&call("Write", serde_json::json!({"path": "a.txt"})), None)
                .await
                .decision,
            PolicyDecision::Deny
        );
    }

    /// `default` 模式对写与命令仍然要问 —— 不能因为放行了只读就把闸门全拆了。
    #[tokio::test]
    async fn default_mode_still_asks_for_writes_and_commands() {
        let engine = engine_in(ApprovalMode::Default);
        for (tool, args) in [
            ("Write", serde_json::json!({"path": "a.txt"})),
            ("Edit", serde_json::json!({"file_path": "a.txt"})),
            ("Bash", serde_json::json!({"command": "git push"})),
            (
                "WebFetch",
                serde_json::json!({"url": "https://example.com"}),
            ),
        ] {
            assert_eq!(
                engine.check(&call(tool, args), None).await.decision,
                PolicyDecision::AskUser,
                "{tool} 必须在 default 模式下弹确认"
            );
        }
    }

    /// 显式 `ask` 规则压得住模式内置放行 —— 用户写 `ask: ["Read"]` 就是"每次都问我"。
    #[tokio::test]
    async fn explicit_ask_rule_beats_the_mode_default_allow() {
        let mut engine = engine_in(ApprovalMode::Default);
        let mut perms = SettingsPermissions::default();
        perms
            .ask
            .push(PermissionRuleSpec::parse("Read").expect("parse"));
        engine.set_settings_permissions(perms);

        let result = engine
            .check(
                &call("Read", serde_json::json!({"file_path": "a.txt"})),
                None,
            )
            .await;
        assert_eq!(result.decision, PolicyDecision::AskUser);
        assert!(
            result.explicit_rule,
            "ask 规则命中时 `explicit_rule` 必须为 true，否则「本会话允许」会把它降级"
        );
        // deny 依然最高优先
        let mut with_deny = engine_in(ApprovalMode::Default);
        let mut perms = SettingsPermissions::default();
        perms
            .deny
            .push(PermissionRuleSpec::parse("Read").expect("parse"));
        with_deny.set_settings_permissions(perms);
        assert!(matches!(
            with_deny
                .check(
                    &call("Read", serde_json::json!({"file_path": "a.txt"})),
                    None
                )
                .await
                .decision,
            PolicyDecision::DenyWithReason(_)
        ));
    }

    /// `acceptEdits` = 读 + 文件编辑 + 只读命令免确认，其余仍问。
    #[tokio::test]
    async fn accept_edits_allows_edits_and_readonly_commands_only() {
        let engine = engine_in(ApprovalMode::AcceptEdits);

        for (tool, args) in [
            ("Read", serde_json::json!({"file_path": "a.rs"})),
            ("Edit", serde_json::json!({"file_path": "a.rs"})),
            ("Write", serde_json::json!({"path": "a.rs"})),
            ("Bash", serde_json::json!({"command": "ls -la src"})),
            (
                "Bash",
                serde_json::json!({"command": "grep -rn TODO src | head -20"}),
            ),
        ] {
            assert_eq!(
                engine.check(&call(tool, args.clone()), None).await.decision,
                PolicyDecision::Allow,
                "{tool}({args}) 在 acceptEdits 下不该弹确认"
            );
        }

        for (tool, args) in [
            ("Bash", serde_json::json!({"command": "git push"})),
            (
                "Bash",
                serde_json::json!({"command": "ls && rm -rf target"}),
            ),
            ("Bash", serde_json::json!({"command": "cat a.txt > b.txt"})),
            (
                "Bash",
                serde_json::json!({"command": "/usr/bin/env sh -c 'rm -rf x'"}),
            ),
            (
                "WebFetch",
                serde_json::json!({"url": "https://example.com"}),
            ),
        ] {
            assert_eq!(
                engine.check(&call(tool, args.clone()), None).await.decision,
                PolicyDecision::AskUser,
                "{tool}({args}) 在 acceptEdits 下必须仍然弹确认"
            );
        }

        // 命令替换类：旧的危险命令检查器直接拒，比弹框更强，所以只断言"绝不静默放行"
        for args in [
            serde_json::json!({"command": "echo $(rm -rf /)"}),
            serde_json::json!({"command": "ls `rm -rf target`"}),
        ] {
            assert_ne!(
                engine
                    .check(&call("Bash", args.clone()), None)
                    .await
                    .decision,
                PolicyDecision::Allow,
                "{args} 绝不能被 acceptEdits 静默放行"
            );
        }
    }

    /// plan 模式对只读工具放行、对写操作直接拒（不是弹框）—— 语义没被新分支改掉。
    #[tokio::test]
    async fn plan_mode_keeps_deny_for_writes() {
        let engine = engine_in(ApprovalMode::Plan);
        assert_eq!(
            engine
                .check(&call("Grep", serde_json::json!({"pattern": "x"})), None)
                .await
                .decision,
            PolicyDecision::Allow
        );
        assert_eq!(
            engine
                .check(&call("Write", serde_json::json!({"path": "a.txt"})), None)
                .await
                .decision,
            PolicyDecision::Deny
        );
    }
}
