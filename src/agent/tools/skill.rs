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
        let user_skill_names = list_user_skill_names(&config.target_dir().clone());
        if !user_skill_names.is_empty() {
            description.push_str(&format!(
                " Prompt-based skills available: {}.",
                user_skill_names.join(", ")
            ));
        }

        Self {
            manager: Arc::new(manager),
            description,
            available_skill_names,
            project_root: config.target_dir().clone(),
        }
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
        }))
    }
}

pub struct SkillInvocation {
    manager: SubAgentManager,
    skill_name: String,
    args: Value,
    /// 项目根 —— 用户自定义 SKILL.md 的查找位置。
    project_root: std::path::PathBuf,
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

        Box::pin(async move {
            if let Some(cb) = update_output.as_ref() {
                cb(format!("Running skill `{}`\n", skill_name));
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
                //
                // 这一段之前是**死代码**：`SkillLoader::load_skills_from_dir`
                // 从来没有调用者，用户目录里的 SKILL.md 写了也不会被加载。
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
                    // 成功加载外部 Skill
                    // 将其内容作为上下文注入返回
                    // 沙箱隔离：明确标记来源和边界
                    // 执行内联 shell 命令 (!(...) 语法)
                    let rendered_body =
                        crate::agent::skills::loader::SkillLoader::render_skill_prompt(&skill);
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
    let dirs = [storage.project_skills_dir(), Storage::user_skills_dir()];

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

        let listed = list_user_skill_names(dir.path());
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

    /// 没有这个 skill 时返回 None（不 panic、不误报）。
    #[tokio::test]
    async fn a_missing_skill_resolves_to_none() {
        let dir = tempfile::tempdir().expect("tmp");
        assert!(load_user_skill_from_dirs("nope", dir.path())
            .await
            .is_none());
    }
}

/// 列出用户目录里的 prompt 型 skill 名字（供工具描述用）。
///
/// 与 [`load_user_skill_from_dirs`] 用同一套目录与命名规则 —— 两个函数
/// 必须一致，否则会出现「描述里说有这个名字，调用时却找不到」。
fn list_user_skill_names(project_root: &std::path::Path) -> Vec<String> {
    use crate::core::config::storage::Storage;

    let storage = Storage::new(project_root.to_path_buf());
    let dirs = [storage.project_skills_dir(), Storage::user_skills_dir()];

    let mut names: Vec<String> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // 形态一：<dir>/<name>/SKILL.md
            if path.is_dir() && path.join("SKILL.md").exists() {
                if !names.iter().any(|n| n == stem) {
                    names.push(stem.to_string());
                }
                continue;
            }
            // 形态二：<dir>/<name>.skill.md
            if path.is_file() && stem.ends_with(".skill") {
                let name = stem.trim_end_matches(".skill").to_string();
                if !names.iter().any(|n| *n == name) {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}
