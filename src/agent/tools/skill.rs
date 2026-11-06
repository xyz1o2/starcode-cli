use crate::agent::skills::{register_custom_subagents, SubAgentManager, SubTask};
use crate::core::tools::{
    BaseDeclarativeTool, Kind, ToolCallConfirmationDetails, ToolError, ToolInvocation,
    ToolLocation, ToolResult as CoreToolResult,
};

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::core::config::Config;
use crate::llm::client::StarClient;

pub struct SkillTool {
    manager: Arc<SubAgentManager>,
    description: String,
    available_skill_names: Vec<String>,
    /// 项目根 —— 用户自定义 SKILL.md（`.star/skills/`）的查找位置。
    project_root: std::path::PathBuf,
    /// 消息总线 —— skill 的 `allowed-tools` / `disallowed-tools` 要写进
    /// 策略引擎（per-turn）。
    message_bus: Option<std::sync::Arc<crate::core::confirmation_bus::MessageBus>>,
    /// 运行中切模型的交接槽 —— skill 的 `model` 字段走它。
    model_switch_cell: Option<crate::agent::model_switch::ModelSwitchCell>,
    /// 后台子代理执行器 —— skill 的 `context: fork` 走它。
    async_runner: Option<std::sync::Arc<crate::agent::subagent::runner::AsyncSubagentRunner>>,
}

impl SkillTool {
    pub fn new(client: StarClient, config: Arc<Config>) -> Self {
        let mut manager = SubAgentManager::new();
        let mut sub_config = (*config).clone();
        sub_config.recursion_depth = sub_config.recursion_depth.saturating_add(1);
        let sub_config = Arc::new(sub_config);

        // 只注册**用户自定义** agent。
        //
        // 内置子代理（analyzer / editor / navigator / auto_fix）已于 2026-11-05
        // 下线 —— 它们的 Rust 实现已从 `src/agent/skills/` 删除。
        // 用户在 `.star/agents/` / `~/.starcode/agents/` 写的定义不受影响。
        let custom_defs = register_custom_subagents(&mut manager, client, sub_config.clone());
        let mut available_skill_names = manager.agent_ids();
        available_skill_names.sort();
        available_skill_names.dedup();
        let mut description = format!(
            "Execute a specialized skill. Available skills: {}. Use this tool to delegate complex tasks to specialized sub-agents.",
            available_skill_names.join(", ")
        );
        if !custom_defs.is_empty() {
            description.push_str(" Includes custom skills loaded from project/user .star/agents.");
        }

        // 用户目录里的 SKILL.md（`.star/skills/`）也算可用 skill。
        //
        // 内置 skill 下线后，这是**唯一**的 prompt 型 skill 来源 ——
        // 不告诉模型有哪些，模型根本不会想到去调它们。
        //
        // 只列**模型可调用**的：`disable-model-invocation: true` 的 skill
        // 不该出现在模型的可用清单里（Claude Code 语义：它的 description
        // 也不进上下文）。
        let user_skills = list_model_callable_skills(&config.target_dir().clone());
        if !user_skills.is_empty() {
            let names: Vec<String> = user_skills.iter().map(|s| s.name.clone()).collect();
            description.push_str(&format!(
                " Prompt-based skills available: {}.",
                names.join(", ")
            ));
        }

        Self {
            manager: Arc::new(manager),
            description,
            available_skill_names,
            project_root: config.target_dir().clone(),
            message_bus: config.runtime_message_bus(),
            model_switch_cell: None,
            async_runner: None,
        }
    }

    fn extras(&self) -> Option<crate::core::tools::tools::ToolExtras> {
        Some(crate::core::tools::tools::ToolExtras::SkillNames(
            self.available_skill_names.clone(),
        ))
    }

    /// 当前可用的 skill 名字（用户自定义 agent + 用户 SKILL.md）。
    ///
    /// 自动触发器要用它判断「这个 skill 到底存不存在」——
    /// 内置 skill 下线后，无条件调 `Skill(skill="analyzer")` 只会白跑一轮
    /// 再报「No suitable agent」。
    pub fn available_skill_names(&self) -> Vec<String> {
        self.available_skill_names.clone()
    }

    /// 注入消息总线（`allowed-tools` 生效所必需）。
    pub fn with_message_bus(
        mut self,
        bus: std::sync::Arc<crate::core::confirmation_bus::MessageBus>,
    ) -> Self {
        self.message_bus = Some(bus);
        self
    }

    /// 注入模型切换槽（`model` 字段生效所必需）。
    pub fn with_model_switch_cell(
        mut self,
        cell: crate::agent::model_switch::ModelSwitchCell,
    ) -> Self {
        self.model_switch_cell = Some(cell);
        self
    }

    /// 注入后台子代理执行器（`context: fork` 生效所必需）。
    pub fn with_async_runner(
        mut self,
        runner: std::sync::Arc<crate::agent::subagent::runner::AsyncSubagentRunner>,
    ) -> Self {
        self.async_runner = Some(runner);
        self
    }
}

impl BaseDeclarativeTool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn display_name(&self) -> &str {
        "Skill Tool"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn kind(&self) -> Kind {
        Kind::Execute
    }

    fn parameter_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": format!(
                        "The name of the skill to execute (available: {})",
                        self.available_skill_names.join(", ")
                    )
                },
                "args": {
                    "type": "object",
                    "description": "Arguments for the skill. Common fields: 'objective', 'target', 'task_type'.",
                    "properties": {
                        "objective": { "type": "string", "description": "The goal of the task" },
                        "task_type": { "type": "string", "description": "Type of task (analyze, edit, search, check, fix)" },
                        "target": { "type": "string", "description": "Target file or context" },
                        "params": { "type": "object", "description": "Additional parameters specific to the skill" }
                    }
                }
            },
            "required": ["skill"]
        })
    }

    fn create_invocation(
        &self,
        params: Value,
    ) -> Result<Box<dyn ToolInvocation>, Box<dyn std::error::Error + Send + Sync>> {
        let skill_name = params
            .get("skill")
            .and_then(|v| v.as_str())
            .ok_or("Missing skill name")?
            .to_string();
        let args = params.get("args").cloned().unwrap_or(json!({}));

        // Dereference the Arc to get the manager (SubAgentManager is Clone)
        Ok(Box::new(SkillInvocation {
            manager: (*self.manager).clone(),
            skill_name,
            args,
            project_root: self.project_root.clone(),
            message_bus: self.message_bus.clone(),
            model_switch_cell: self.model_switch_cell.clone(),
            async_runner: self.async_runner.clone(),
        }))
    }
}

pub struct SkillInvocation {
    manager: SubAgentManager,
    skill_name: String,
    args: Value,
    /// 项目根 —— 用户自定义 SKILL.md 的查找位置。
    project_root: std::path::PathBuf,
    /// 消息总线（per-turn 工具策略）。
    message_bus: Option<std::sync::Arc<crate::core::confirmation_bus::MessageBus>>,
    /// 模型切换槽。
    model_switch_cell: Option<crate::agent::model_switch::ModelSwitchCell>,
    /// 后台子代理执行器（`context: fork`）。
    async_runner: Option<std::sync::Arc<crate::agent::subagent::runner::AsyncSubagentRunner>>,
}

#[async_trait]
impl ToolInvocation for SkillInvocation {
    fn get_description(&self) -> String {
        format!("Executing skill: {}", self.skill_name)
    }

    fn tool_locations(&self) -> Vec<ToolLocation> {
        vec![]
    }

    fn execute(
        &self,
        _signal: Option<&tokio_util::sync::CancellationToken>,
        update_output: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<CoreToolResult, Box<dyn std::error::Error>>>
                + Send
                + '_,
        >,
    > {
        let manager = self.manager.clone();
        let skill_name = self.skill_name.clone();
        let args = self.args.clone();
        let update_output = update_output.clone();
        let project_root = self.project_root.clone();
        let message_bus = self.message_bus.clone();
        let model_switch_cell = self.model_switch_cell.clone();
        let async_runner = self.async_runner.clone();

        Box::pin(async move {
            if let Some(cb) = update_output.as_ref() {
                cb(format!("Running skill `{skill_name}`\n"));
            }

            // Construct SubTask
            let objective = args
                .get("objective")
                .and_then(|v| v.as_str())
                .unwrap_or(&skill_name)
                .to_string();
            let task_type = args
                .get("task_type")
                .and_then(|v| v.as_str())
                .unwrap_or(&skill_name)
                .to_string();
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = args
                .get("params")
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();

            let mut task = SubTask::new(
                uuid::Uuid::new_v4().to_string(),
                objective,
                task_type,
                target,
            );

            for (k, v) in params {
                task = task.with_param(k, v);
            }

            // Try to find agent by ID first, then by task matching
            let agent = if let Some(agent) = manager.get_agent(&skill_name) {
                Some(agent)
            } else {
                manager.select_agent(&task)
            };

            if let Some(agent) = agent {
                match agent.execute(task).await {
                    Ok(result) => {
                        let output = if result.success {
                            result.summary
                        } else {
                            format!(
                                "Skill execution failed: {}",
                                result.error.clone().unwrap_or_default()
                            )
                        };

                        let mut details = String::new();
                        if let Some(d) = result.details {
                            details.push_str(&format!("\nDetails:\n{}", d));
                        }
                        if let Some(data) = result.data {
                            details.push_str(&format!(
                                "\nData:\n{}",
                                serde_json::to_string_pretty(&data).unwrap_or_default()
                            ));
                        }

                        let final_output = format!("{}{}", output, details);
                        if let Some(cb) = update_output.as_ref() {
                            cb(format!("Skill `{}` completed\n", skill_name));
                        }

                        Ok(CoreToolResult {
                            llm_content: Some(final_output.clone()),
                            return_display: Some(final_output.clone()),
                            output: final_output,
                            error: if result.success {
                                None
                            } else {
                                Some(ToolError {
                                    error_type: "ExecutionFailed".to_string(),
                                    message: result.error.unwrap_or_default(),
                                })
                            },
                            data: None,
                        })
                    }
                    Err(e) => Ok(CoreToolResult {
                        llm_content: Some(format!("Skill execution error: {}", e)),
                        return_display: Some(format!("Skill execution error: {}", e)),
                        output: format!("Skill execution error: {}", e),
                        error: Some(ToolError {
                            error_type: "RuntimeError".to_string(),
                            message: e.to_string(),
                        }),
                        data: None,
                    }),
                }
            } else {
                // ============ 用户自定义 SKILL.md（`.star/skills/`） ============
                //
                // 内置 skill 已于 2026-11-05 下线，但**用户自己写的**
                // SKILL.md 必须照样能用 —— 这是本次删除的前提。
                //
                // 查找顺序（先命中先用）：
                // 1. 项目 `.star/skills/<name>/SKILL.md` 或 `<name>.skill.md`
                // 2. 用户 `~/.starcode/skills/<name>/SKILL.md`（全局）
                let skill_def = load_user_skill_from_dirs(&skill_name, &project_root).await;

                // ============ 外部 Skill 加载 (GitHub / Local) ============
                // 用户目录里没有，再试 GitHub / 本地路径。

                let skill_def = match skill_def {
                    Some(found) => Some(found),
                    None if skill_name.starts_with("http") || skill_name.starts_with("git@") => {
                        // GitHub / Remote Git
                        crate::agent::skills::loader::SkillLoader::load_skill_from_github(
                            &skill_name,
                            None,
                        )
                        .await
                    }
                    None => {
                        // Local Path
                        let path = std::path::Path::new(&skill_name);
                        if path.exists() {
                            crate::agent::skills::loader::SkillLoader::load_skill_from_file(path)
                                .await
                        } else {
                            None
                        }
                    }
                };

                if let Some(skill) = skill_def {
                    // ── 调用闸门：`disable-model-invocation` ──────────────────
                    //
                    // Claude Code 语义：模型不得自动调用这个 skill，用户仍可
                    // 显式调用（`/name`）。
                    //
                    // StarCode 没有 `/name` 入口（用户 skill 不在 `/` 菜单里），
                    // 而 `SkillTool` 的调用**全部**来自模型 —— 所以对这个工具
                    // 而言，"模型不许自动调" 就等于"不许调"。用户想用就走
                    // `/skills show` 看内容。
                    if !skill.metadata.model_may_invoke() {
                        let output = format!(
                            "Skill `{}` is not available for model invocation \
                             (its frontmatter sets `disable-model-invocation: true`). \
                             Use `/skills show {}` to read it.",
                            skill.name, skill.name
                        );
                        return Ok(CoreToolResult {
                            llm_content: Some(output.clone()),
                            return_display: Some(output.clone()),
                            output,
                            error: Some(ToolError {
                                error_type: "SkillNotInvocable".to_string(),
                                message: format!(
                                    "skill `{}` sets disable-model-invocation: true",
                                    skill.name
                                ),
                            }),
                            data: None,
                        });
                    }

                    // ── `context: fork`：在隔离子代理里跑 ─────────────────────
                    //
                    // Claude Code 语义：skill 正文成为子代理的 prompt，
                    // 子代理**看不到**当前对话历史 —— 所以 skill 正文必须
                    // 自成一体。默认后台运行（`background: false` 则本轮等结果）。
                    if skill.metadata.is_forked() {
                        return run_skill_forked(&skill, async_runner.as_ref(), &skill_name).await;
                    }

                    // ── 应用 frontmatter 的运行时效果 ─────────────────────────
                    //
                    // `allowed-tools` / `disallowed-tools` 写进策略引擎的
                    // per-turn 表（下一条用户消息即清）；`model` 发布一次
                    // 模型切换（`execute_turn` 在下次 provider 请求前 drain）。
                    apply_skill_runtime_effects(
                        &skill,
                        message_bus.as_ref(),
                        model_switch_cell.as_ref(),
                    )
                    .await;

                    // 成功加载外部 Skill
                    // 将其内容作为上下文注入返回
                    // 沙箱隔离：明确标记来源和边界
                    // 执行内联 shell 命令 (!(...) 语法)
                    //
                    // 参数替换：frontmatter 的 `arguments` 声明按位置取
                    // `args` 里的值（`$name` / `$1` / `$ARGUMENTS`）。
                    let rendered_body = render_skill_with_args(&skill, &args);
                    let output = format!(
                        "✅ External Skill Loaded Successfully\n\n<skill_sandbox name=\"{}\" source=\"{}\">\n{}\n</skill_sandbox>\n\n⚠️ This skill is running in a restricted context. Please follow the instructions above.",
                        skill.name, skill.location, rendered_body
                    );

                    Ok(CoreToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(output.clone()),
                        output,
                        error: None,
                        data: None,
                    })
                } else {
                    let output = format!(
                        "No suitable agent or external skill found for: {}",
                        skill_name
                    );
                    Ok(CoreToolResult {
                        llm_content: Some(output.clone()),
                        return_display: Some(output.clone()),
                        output: output.clone(),
                        error: Some(ToolError {
                            error_type: "NoAgent".to_string(),
                            message: output,
                        }),
                        data: None,
                    })
                }
            }
        })
    }
}

/// 在**用户目录**里找同名 skill。
///
/// 查找位置（先命中先用，项目优先于全局）：
/// 1. `<project>/.star/skills/<name>/SKILL.md`
/// 2. `<project>/.star/skills/<name>.skill.md`
/// 3. `~/.starcode/skills/<name>/SKILL.md`
/// 4. `~/.starcode/skills/<name>.skill.md`
/// 5. `<project>/.star/extensions/skills/<name>/SKILL.md`（`/extension` 装的）
///
/// 目录布局与 Claude Code 一致（`<skill-dir>/SKILL.md`），另接受
/// `<name>.skill.md` 这种单文件写法 —— `SkillLoader::load_skills_from_dir`
/// 认这两种形态，这里对单个名字做同样的事。
///
/// 名字里的分隔符（`-` / `_` / 空格）做等价匹配：`my-skill` 也能命中
/// 目录 `my_skill`。这是用户手写目录名时最容易踩的一步。
async fn load_user_skill_from_dirs(
    name: &str,
    project_root: &std::path::Path,
) -> Option<crate::agent::skills::loader::SkillDefinition> {
    use crate::agent::skills::loader::SkillLoader;
    use crate::core::config::storage::Storage;

    let storage = Storage::new(project_root.to_path_buf());
    // 三个来源：项目 / 全局 / 扩展安装（见 `Storage::skill_roots`）。
    //
    // 扩展这一层以前没人扫 —— marketplace 装好的 skill 落在
    // `.star/extensions/skills/<name>/SKILL.md`，agent 看不到，于是
    // 「装完的 skill 永远不被调用」。
    let dirs = storage.skill_roots();

    // 名字的等价变体：原名 + 分隔符互换。
    let mut candidates: Vec<String> = vec![name.to_string()];
    for sep in ['-', '_', ' '] {
        candidates.push(name.replace(sep, "_"));
        candidates.push(name.replace(sep, "-"));
    }
    candidates.sort();
    candidates.dedup();

    for dir in dirs {
        for candidate in &candidates {
            // 形态一：<dir>/<name>/SKILL.md
            let nested = dir.join(candidate).join("SKILL.md");
            if nested.exists() {
                if let Some(skill) = SkillLoader::load_skill_from_file(&nested).await {
                    return Some(skill);
                }
            }
            // 形态二：<dir>/<name>.skill.md
            let flat = dir.join(format!("{candidate}.skill.md"));
            if flat.exists() {
                if let Some(skill) = SkillLoader::load_skill_from_file(&flat).await {
                    return Some(skill);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用户自定义 SKILL.md 必须能被找到 —— 这是「删内置、保用户」的底线。
    ///
    /// 内置 skill 下线后，用户在 `.star/skills/<name>/SKILL.md` 写的 skill
    /// 是**唯一**的 skill 来源。这条测试守住那条查找路径。
    #[tokio::test]
    async fn a_user_defined_skill_is_found_in_the_project_skills_dir() {
        let dir = tempfile::tempdir().expect("tmp");
        let skills_dir = dir.path().join(".star").join("skills").join("review");
        std::fs::create_dir_all(&skills_dir).expect("mkdir");
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: review\ndescription: 审查代码\n---\n\n逐行审查。\n",
        )
        .expect("write");

        let found = load_user_skill_from_dirs("review", dir.path()).await;
        assert!(
            found.is_some(),
            "用户写的 .star/skills/review/SKILL.md 必须能被找到"
        );
        assert_eq!(found.expect("found").name, "review");
    }

    /// 分隔符等价：`my-skill` 也能命中目录 `my_skill`。
    ///
    /// 用户手写目录名时最容易踩这一步（命令里写 `my-skill`，目录建的是
    /// `my_skill`），值得单独锁住。
    #[tokio::test]
    async fn a_skill_is_found_regardless_of_separator_style() {
        let dir = tempfile::tempdir().expect("tmp");
        let skills_dir = dir.path().join(".star").join("skills").join("my_skill");
        std::fs::create_dir_all(&skills_dir).expect("mkdir");
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: my_skill\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");

        assert!(
            load_user_skill_from_dirs("my-skill", dir.path())
                .await
                .is_some(),
            "my-skill 应该能命中目录 my_skill"
        );
        assert!(
            load_user_skill_from_dirs("my_skill", dir.path())
                .await
                .is_some(),
            "my_skill 应该能命中目录 my_skill"
        );
    }

    /// 单文件形态 `<name>.skill.md` 也要认。
    #[tokio::test]
    async fn a_flat_skill_file_is_found() {
        let dir = tempfile::tempdir().expect("tmp");
        let skills_dir = dir.path().join(".star").join("skills");
        std::fs::create_dir_all(&skills_dir).expect("mkdir");
        std::fs::write(
            skills_dir.join("audit.skill.md"),
            "---\nname: audit\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");

        assert!(
            load_user_skill_from_dirs("audit", dir.path())
                .await
                .is_some(),
            ".star/skills/audit.skill.md 必须能被找到"
        );
    }

    /// 「描述里列出来的名字」必须真的能被找到。
    ///
    /// 两个函数必须一致 —— 不一致就会出现「工具描述说有 `review`，模型调了
    /// 却报 no suitable agent」，而这正是内置 skill 下线后唯一的 skill 来源，
    /// 错了就等于用户写的 skill 全都不可用。
    #[tokio::test]
    async fn every_listed_skill_name_resolves_to_a_skill() {
        let dir = tempfile::tempdir().expect("tmp");
        let skills_dir = dir.path().join(".star").join("skills");
        std::fs::create_dir_all(skills_dir.join("review")).expect("mkdir");
        std::fs::write(
            skills_dir.join("review").join("SKILL.md"),
            "---
name: review\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");
        std::fs::write(
            skills_dir.join("audit.skill.md"),
            "---\nname: audit\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");

        let listed: Vec<String> = list_model_callable_skills(dir.path())
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(
            listed.contains(&"review".to_string()),
            "review 必须出现在清单里：{listed:?}"
        );
        assert!(
            listed.contains(&"audit".to_string()),
            "audit 必须出现在清单里：{listed:?}"
        );

        for name in &listed {
            assert!(
                load_user_skill_from_dirs(name, dir.path()).await.is_some(),
                "清单里的 `{name}` 必须能被找到（描述与查找必须一致）"
            );
        }
    }

    /// **扩展安装的 skill 必须能被找到**（断链回归）。
    ///
    /// `/extension` marketplace 装好的 skill 落在
    /// `.star/extensions/skills/<name>/SKILL.md`，而 agent 侧以前只看
    /// `.star/skills/` 与 `~/.starcode/skills/` —— 于是「装完的 skill 永远
    /// 不会被模型调用」，`/skills list` 里也看不见。
    ///
    /// 三个来源现在由 `Storage::skill_roots()` 统一给出
    /// （与 Claude Code 的 user / project / plugin 三层扫描一致）。
    #[tokio::test]
    async fn an_extension_installed_skill_is_found() {
        let dir = tempfile::tempdir().expect("tmp");
        let ext_skills = dir.path().join(".star").join("extensions").join("skills");
        std::fs::create_dir_all(ext_skills.join("review")).expect("mkdir");
        std::fs::write(
            ext_skills.join("review").join("SKILL.md"),
            "---\nname: review\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");

        let found = load_user_skill_from_dirs("review", dir.path()).await;
        assert!(
            found.is_some(),
            "扩展安装的 skill 必须能被找到（marketplace 装完就该能用）"
        );
        assert_eq!(found.expect("found").name, "review");
    }

    /// 扩展来源也要出现在**清单**里 —— 否则模型看不到它的存在。
    #[test]
    fn an_extension_installed_skill_is_listed() {
        let dir = tempfile::tempdir().expect("tmp");
        let ext_skills = dir.path().join(".star").join("extensions").join("skills");
        std::fs::create_dir_all(ext_skills.join("review")).expect("mkdir");
        std::fs::write(
            ext_skills.join("review").join("SKILL.md"),
            "---\nname: review\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");

        let listed: Vec<String> = list_model_callable_skills(dir.path())
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(
            listed.contains(&"review".to_string()),
            "扩展装的 skill 必须出现在清单里：{listed:?}"
        );
    }

    /// 没有这个 skill 时返回 None（不 panic、不误报）。
    #[tokio::test]
    async fn a_missing_skill_resolves_to_none() {
        let dir = tempfile::tempdir().expect("tmp");
        assert!(load_user_skill_from_dirs("nope", dir.path())
            .await
            .is_none());
    }
}

/// 列出用户目录里的 prompt 型 skill（供工具描述用）。
///
/// 与 [`load_user_skill_from_dirs`] 用同一套目录与命名规则 —— 两个函数
/// 必须一致，否则会出现「描述里说有这个名字，调用时却找不到」。
///
/// **只返回模型可调用的**：`disable-model-invocation: true` 的 skill
/// 不进模型的可用清单（Claude Code 语义：它的 description 也不进上下文，
/// 模型看不到它，也就不会想去调它）。
///
/// 这里用**同步**的文件读取（不是 `load_skills_from_dir`）：工具描述在
/// 构造期就要定下来，而 `BaseDeclarativeTool::description(&self) -> &str`
/// 是同步的。skill 文件很小（几 KB 的 markdown），同步读没有代价；
/// 换成 async 就得在同步上下文里造 runtime，徒增复杂度。
fn list_model_callable_skills(
    project_root: &std::path::Path,
) -> Vec<crate::agent::skills::loader::SkillDefinition> {
    use crate::agent::skills::loader::SkillLoader;
    use crate::core::config::storage::Storage;

    let storage = Storage::new(project_root.to_path_buf());
    // 三个来源（项目 / 全局 / 扩展安装）—— 见 `Storage::skill_roots`。
    let dirs = storage.skill_roots();

    let mut skills: Vec<crate::agent::skills::loader::SkillDefinition> = Vec::new();
    for dir in dirs {
        for skill in load_skills_in_dir_sync(&dir) {
            // 项目目录优先：同名时用户目录的版本不重复列。
            if skills.iter().any(|s| s.name == skill.name) {
                continue;
            }
            skills.push(skill);
        }
    }
    skills.retain(|s| s.metadata.model_may_invoke());
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// 同步读一个目录下的所有 skill（`<name>/SKILL.md` 与 `<name>.skill.md`）。
fn load_skills_in_dir_sync(
    dir: &std::path::Path,
) -> Vec<crate::agent::skills::loader::SkillDefinition> {
    use crate::agent::skills::loader::SkillLoader;

    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file = if path.is_dir() {
            let candidate = path.join("SKILL.md");
            if candidate.exists() {
                Some(candidate)
            } else {
                None
            }
        } else {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|name| name.ends_with(".skill.md"))
                .unwrap_or(false)
                .then(|| path.clone())
        };
        let Some(file) = file else { continue };
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        let (metadata, body) = SkillLoader::parse_frontmatter(&content);
        let name = if metadata.name.is_empty() {
            path.file_stem()
                .map(|s| s.to_string_lossy().trim_end_matches(".skill").to_string())
                .unwrap_or_default()
        } else {
            metadata.name.clone()
        };
        if name.is_empty() {
            continue;
        }
        skills.push(crate::agent::skills::loader::SkillDefinition {
            name,
            description: metadata.description.clone(),
            location: file.display().to_string(),
            body,
            disabled: metadata.disabled,
            version: metadata.version.clone(),
            metadata,
        });
    }
    skills
}

/// 同步加载一个目录下的所有 skill。
///
/// `SkillLoader::load_skills_from_dir` 是 async 的，而工具描述的构造
/// 是同步的（`BaseDeclarativeTool::description(&self) -> &str`）。
///
/// 这里用一条独立的 runtime 跑 —— 目录扫描是纯文件 IO，不碰 tokio 的
/// 调度器，不会与当前线程的 runtime 死锁（`block_on` 只在**没有**正在
/// 运行的 runtime 时才安全，见 [`crate::utils::async_block_on`]）。
async fn block_on_load_skills(
    dir: &std::path::Path,
) -> Vec<crate::agent::skills::loader::SkillDefinition> {
    crate::agent::skills::loader::SkillLoader::load_skills_from_dir(dir).await
}

/// 把 skill frontmatter 的**运行时**效果落到系统里。
///
/// 三个字段有运行时含义（其余字段要么是纯展示，要么本次不实现）：
///
/// - `allowed-tools` / `disallowed-tools` → 策略引擎的 per-turn 表。
///   **只到下一条用户消息为止**（Claude Code 语义：grant clears when you
///   send your next message）。写入的是 `PolicyEngine`，判定走
///   `PolicyEngine::check` —— 与用户写在 `settings.json` 里的 `allow` 规则
///   同一个入口，不会绕过 deny 规则。
///
/// - `model` → 发布一次模型切换。`execute_turn` 在每次 provider 请求前
///   drain 这个槽，所以效果是「本轮剩余部分用新模型」。
///   `inherit` 表示保持当前模型（Claude Code 的约定）。
///
/// `effort` 没有实现：StarCode 的 effort 只有 `self_tuning` 那条「模型自己
/// 调」的通路，没有 per-turn override 的落点，硬塞会与那套语义混淆。
/// `context: fork` 由调用方单独处理（要起子代理，不只是改状态）。
async fn apply_skill_runtime_effects(
    skill: &crate::agent::skills::loader::SkillDefinition,
    message_bus: Option<&std::sync::Arc<crate::core::confirmation_bus::MessageBus>>,
    model_switch_cell: Option<&crate::agent::model_switch::ModelSwitchCell>,
) {
    let meta = &skill.metadata;

    // ── allowed-tools / disallowed-tools ──────────────────────────
    if !meta.allowed_tools.is_empty() || !meta.disallowed_tools.is_empty() {
        if let Some(bus) = message_bus {
            bus.apply_turn_tool_policy(&meta.allowed_tools, &meta.disallowed_tools)
                .await;
            crate::utils::logging::append_debug_log_line(&format!(
                "[Skill] `{}` applied turn tool policy: allowed={:?} disallowed={:?}",
                skill.name, meta.allowed_tools, meta.disallowed_tools
            ));
        } else {
            crate::utils::logging::append_debug_log_line(&format!(
                "[Skill] `{}` declares allowed-tools but no message bus is wired — \
                 the grant is not applied",
                skill.name
            ));
        }
    }

    // ── effort ───────────────────────────────────────────────────
    //
    // 本轮档位（skill 显式要求）压过会话档位，但只到下一条用户消息。
    if let Some(raw) = meta.effort.as_deref() {
        if let Some(effort) = crate::types::ThinkingEffort::parse_skill_effort(raw) {
            crate::llm::thinking::set_turn_effort(&effort);
            crate::utils::logging::append_debug_log_line(&format!(
                "[Skill] `{}` set this turn's effort to `{}`",
                skill.name,
                effort.as_str()
            ));
        }
    }

    // ── model ────────────────────────────────────────────────────
    if let Some(model) = meta.model.as_deref() {
        let trimmed = model.trim();
        // `inherit` = 保持当前模型（Claude Code 的约定）。
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("inherit") {
            if let Some(cell) = model_switch_cell {
                crate::agent::model_switch::publish(
                    cell,
                    crate::agent::model_switch::PendingModelSwitch {
                        model: trimmed.to_string(),
                        provider_id: None,
                    },
                )
                .await;
                crate::utils::logging::append_debug_log_line(&format!(
                    "[Skill] `{}` switched the rest of this turn to model `{trimmed}`",
                    skill.name
                ));
            }
        }
    }
}

/// 在隔离子代理里跑一个 skill（`context: fork`）。
///
/// Claude Code 语义：
/// - skill 正文成为子代理的 prompt；子代理**看不到**当前对话历史，
///   所以 skill 必须自成一体。
/// - `agent` 指定子代理类型（默认 general-purpose）。
/// - `background` 指定是否后台运行（默认 true = 后台）。
///
/// 返回给模型的是「子代理跑完了」这件事，不是 skill 正文 —— 正文已经被
/// 子代理吃掉了，再塞回主上下文就是重复。
async fn run_skill_forked(
    skill: &crate::agent::skills::loader::SkillDefinition,
    async_runner: Option<&std::sync::Arc<crate::agent::subagent::runner::AsyncSubagentRunner>>,
    skill_name: &str,
) -> Result<CoreToolResult, Box<dyn std::error::Error>> {
    let Some(runner) = async_runner else {
        let output = format!(
            "Skill `{}` requests `context: fork`, but no subagent runner is available \
             in this session. Run it without `context: fork`.",
            skill.name
        );
        return Ok(CoreToolResult {
            llm_content: Some(output.clone()),
            return_display: Some(output.clone()),
            output,
            error: Some(ToolError {
                error_type: "ForkUnavailable".to_string(),
                message: "no subagent runner available for context: fork".to_string(),
            }),
            data: None,
        });
    };

    // 子代理类型：`agent` 字段，缺省 general-purpose。
    let subagent_type = match skill.metadata.agent.as_deref() {
        Some(name) => serde_json::from_value::<crate::core::agents::SubagentType>(
            serde_json::Value::String(normalize_subagent_type_slug(name)),
        )
        .unwrap_or(crate::core::agents::SubagentType::GeneralPurpose),
        None => crate::core::agents::SubagentType::GeneralPurpose,
    };

    // 子代理的 prompt = skill 正文（`!cmd` 块先执行）。
    let prompt = crate::agent::skills::loader::SkillLoader::render_skill_prompt(skill);

    let request =
        crate::core::agents::SubAgentRequest::new(prompt).with_subagent_type(subagent_type.clone());

    let type_label = subagent_type.user_facing_name().to_string();
    let description = format!("skill `{}`", skill.name);
    let agent_name = Some(skill_name.to_string());

    if skill.metadata.fork_in_background() {
        // 后台：立刻返回，结果随通知回流主会话。
        let launched = runner.spawn_background(request, agent_name, description, type_label);
        let output = format!(
            "Skill `{}` is running in a forked subagent ({}). Its result will arrive \
             in this conversation when it finishes.",
            skill.name, launched.agent_id
        );
        Ok(CoreToolResult {
            llm_content: Some(output.clone()),
            return_display: Some(output.clone()),
            output,
            error: None,
            data: Some(serde_json::json!({
                "status": "fork_launched",
                "agent_id": launched.agent_id,
                "skill": skill.name,
            })),
        })
    } else {
        // 前台：本轮等结果。
        let result = runner
            .run_foreground(request, agent_name, description, type_label)
            .await
            .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;
        let output = if result.output.trim().is_empty() {
            format!("Skill `{}` finished with no output.", skill.name)
        } else {
            result.output.clone()
        };
        Ok(CoreToolResult {
            llm_content: Some(output.clone()),
            return_display: Some(output.clone()),
            output,
            error: None,
            data: Some(serde_json::json!({
                "status": "fork_completed",
                "skill": skill.name,
                "tool_use_count": result.tool_use_count,
                "total_tokens": result.total_tokens,
            })),
        })
    }
}

/// 把 frontmatter 里的 agent 名归一成 `SubagentType` 的 snake_case slug。
///
/// Claude Code 那边写的是 `Explore` / `Plan` / `general-purpose` 这种
/// 展示名，这里要转成 serde 能认的 slug（`explorer` / `general_purpose`）。
/// 归一本身收敛在 `SubagentType::from_name`（枚举自己是身份的唯一事实来源）；
/// 这里只保留"未识别时透传原名"的兜底，自定义 agent 靠它往下走。
fn normalize_subagent_type_slug(name: &str) -> String {
    crate::core::agents::SubagentType::from_name(name)
        .map(|t| t.as_str().to_string())
        .unwrap_or_else(|| name.trim().to_owned())
}

/// 渲染 skill 正文：执行 `!cmd` 块 + 按 frontmatter 声明替换参数。
///
/// 位置参数从工具调用的 `args` 里取 —— CC 的约定是 `arguments` 声明的顺序
/// 对应位置参数的顺序（`arguments: [issue, branch]` → 第 1 个值给 `$issue`）。
///
/// `args` 的形状有两种：
/// - 字符串 → 整个当作 `$ARGUMENTS`（单参数 skill 的常见写法）
/// - 数组 → 按顺序映射到声明的参数名
fn render_skill_with_args(
    skill: &crate::agent::skills::loader::SkillDefinition,
    args: &serde_json::Value,
) -> String {
    let positional = positional_args_from(args);
    let body = crate::agent::skills::loader::SkillLoader::render_skill_prompt(skill);
    crate::agent::skills::loader::SkillLoader::substitute_skill_arguments(
        &body,
        &skill.metadata.arguments,
        &positional,
    )
}

/// 从工具调用的 `args` 里取出位置参数（字符串数组）。
fn positional_args_from(args: &serde_json::Value) -> Vec<String> {
    match args {
        serde_json::Value::Array(items) => items
            .iter()
            .map(|item| match item {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        serde_json::Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// frontmatter 语义的端到端测试。
///
/// 这组测试走**生产接线**（真实的 `PolicyEngine` + `MessageBus`），
/// 验证的不是「解析出来了」而是「**真的生效了**」。
#[cfg(test)]
mod frontmatter_effects {
    use super::*;
    use crate::core::confirmation_bus::MessageBus;
    use crate::core::policy::{PolicyDecision, PolicyEngine};

    fn bus() -> std::sync::Arc<MessageBus> {
        std::sync::Arc::new(MessageBus::new(
            PolicyEngine::new(Default::default()),
            false,
        ))
    }

    fn call(name: &str) -> crate::core::policy::FunctionCall {
        crate::core::policy::FunctionCall {
            name: name.to_string(),
            args: Some(serde_json::json!({})),
            read_only: None,
        }
    }

    /// `allowed-tools` → 该工具本轮免确认。
    ///
    /// 走真实的 `MessageBus` → `PolicyEngine::check` 路径 ——
    /// 不是直接调 `apply_turn_tool_policy`（那会绕开接线）。
    #[tokio::test]
    async fn allowed_tools_take_effect_through_the_message_bus() {
        let bus = bus();
        bus.apply_turn_tool_policy(&["Bash".to_string()], &[]).await;

        let decision = bus.check_tool_policy(&call("Bash"), None).await.decision;
        assert_eq!(
            decision,
            PolicyDecision::Allow,
            "skill 的 allowed-tools 应当真的让 Bash 免确认"
        );

        // 不在名单里的照常要确认。
        let decision = bus.check_tool_policy(&call("Write"), None).await.decision;
        assert_eq!(
            decision,
            PolicyDecision::AskUser,
            "不在名单里的工具必须照常弹确认"
        );
    }

    /// 下一条用户消息 → 授权清空（生产路径上由 `agent_run` 调用）。
    #[tokio::test]
    async fn the_grant_is_cleared_for_the_next_user_message() {
        let bus = bus();
        bus.apply_turn_tool_policy(&["Bash".to_string()], &[]).await;
        assert_eq!(
            bus.check_tool_policy(&call("Bash"), None).await.decision,
            PolicyDecision::Allow
        );

        bus.clear_turn_tool_policy().await;

        assert_eq!(
            bus.check_tool_policy(&call("Bash"), None).await.decision,
            PolicyDecision::AskUser,
            "下一条用户消息后授权必须失效"
        );
    }

    /// `disallowed-tools` → 本轮禁用，且压过「本会话总是允许」。
    #[tokio::test]
    async fn disallowed_tools_take_effect_through_the_message_bus() {
        let bus = bus();
        bus.apply_turn_tool_policy(&[], &["AskUserQuestion".to_string()])
            .await;

        let decision = bus
            .check_tool_policy(&call("AskUserQuestion"), None)
            .await
            .decision;
        assert!(
            matches!(decision, PolicyDecision::DenyWithReason(_)),
            "disallowed-tools 里的工具本轮必须被拒，实际 {decision:?}"
        );
    }

    /// `disable-model-invocation` 的 skill 不进模型的可用清单。
    ///
    /// 这是「模型看不到它，也就不会想去调它」的那一层。
    #[test]
    fn a_gated_skill_is_not_offered_to_the_model() {
        let dir = tempfile::tempdir().expect("tmp");
        let skills_dir = dir.path().join(".star").join("skills");

        std::fs::create_dir_all(skills_dir.join("open_skill")).expect("mkdir");
        std::fs::write(
            skills_dir.join("open_skill").join("SKILL.md"),
            "---\nname: open_skill\ndescription: d\n---\n\nbody\n",
        )
        .expect("write");

        std::fs::create_dir_all(skills_dir.join("gated")).expect("mkdir");
        std::fs::write(
            skills_dir.join("gated").join("SKILL.md"),
            "---\nname: gated\ndescription: d\ndisable-model-invocation: true\n---\n\nbody\n",
        )
        .expect("write");

        let listed = list_model_callable_skills(dir.path());
        let names: Vec<&str> = listed.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"open_skill"),
            "普通 skill 必须在清单里：{names:?}"
        );
        assert!(
            !names.contains(&"gated"),
            "disable-model-invocation 的 skill 不该出现在模型清单里：{names:?}"
        );
    }

    /// `agent:` 字段 → 子代理类型。
    ///
    /// CC 那边写的是展示名（`Explore` / `Plan` / `general-purpose`），
    /// 这里要归一成 `SubagentType` 的 serde slug。
    #[test]
    fn the_agent_field_maps_to_a_subagent_type() {
        assert_eq!(normalize_subagent_type_slug("Explore"), "explorer");
        assert_eq!(
            normalize_subagent_type_slug("general-purpose"),
            "general_purpose"
        );
        assert_eq!(
            normalize_subagent_type_slug("Code Reviewer"),
            "code_reviewer"
        );
        // 归一后的 slug 必须能被 serde 认出来。
        let parsed: Result<crate::core::agents::SubagentType, _> = serde_json::from_value(
            serde_json::Value::String(normalize_subagent_type_slug("Explore")),
        );
        assert!(parsed.is_ok(), "归一后的 slug 必须是合法变体");
    }

    /// `model: inherit` → 不换模型（CC 的约定）。
    #[tokio::test]
    async fn model_inherit_does_not_switch() {
        let cell = crate::agent::model_switch::new_cell();
        let skill = crate::agent::skills::loader::SkillDefinition {
            name: "s".to_string(),
            description: String::new(),
            location: "/tmp/s".to_string(),
            body: "body".to_string(),
            disabled: false,
            version: None,
            metadata: {
                let mut meta = crate::agent::skills::loader::SkillMetadata::default();
                meta.model = Some("inherit".to_string());
                meta
            },
        };

        apply_skill_runtime_effects(&skill, None, Some(&cell)).await;

        assert!(
            crate::agent::model_switch::take(&cell).await.is_none(),
            "`inherit` 不该发布模型切换"
        );
    }

    /// `model: gpt-4o` → 发布一次模型切换（本轮剩余部分生效）。
    #[tokio::test]
    async fn a_model_field_publishes_a_switch() {
        let cell = crate::agent::model_switch::new_cell();
        let skill = crate::agent::skills::loader::SkillDefinition {
            name: "s".to_string(),
            description: String::new(),
            location: "/tmp/s".to_string(),
            body: "body".to_string(),
            disabled: false,
            version: None,
            metadata: {
                let mut meta = crate::agent::skills::loader::SkillMetadata::default();
                meta.model = Some("gpt-4o".to_string());
                meta
            },
        };

        apply_skill_runtime_effects(&skill, None, Some(&cell)).await;

        let pending = crate::agent::model_switch::take(&cell).await;
        assert!(pending.is_some(), "`model` 字段必须发布一次模型切换");
        assert_eq!(pending.expect("pending").model, "gpt-4o");
    }

    /// `effort` → 本轮档位（压过会话档位），下一条消息后清空。
    #[tokio::test]
    async fn an_effort_field_sets_the_turn_effort() {
        crate::llm::thinking::set_session_effort(&crate::types::ThinkingEffort::Low);
        crate::llm::thinking::clear_turn_effort();

        let skill = crate::agent::skills::loader::SkillDefinition {
            name: "s".to_string(),
            description: String::new(),
            location: "/tmp/s".to_string(),
            body: "body".to_string(),
            disabled: false,
            version: None,
            metadata: {
                let mut meta = crate::agent::skills::loader::SkillMetadata::default();
                meta.effort = Some("high".to_string());
                meta
            },
        };

        apply_skill_runtime_effects(&skill, None, None).await;
        assert_eq!(
            crate::llm::thinking::current_effort(),
            crate::types::ThinkingEffort::High,
            "本轮档位必须压过会话档位"
        );

        // 下一条用户消息 → 回到会话档位。
        crate::llm::thinking::clear_turn_effort();
        assert_eq!(
            crate::llm::thinking::current_effort(),
            crate::types::ThinkingEffort::Low,
            "下一条用户消息后必须回到会话档位"
        );
    }
}
