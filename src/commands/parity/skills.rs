//! parity 命令族 —— skills（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::on_off;
use crate::commands::parity::core::push_msg;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// 载入**所有来源**的 SKILL.md（项目 / 全局 / 扩展安装）。
///
/// 三个根目录由 `Storage::skill_roots()` 统一给出 —— 与 agent 侧、
/// `/skills list`、`/reload-skills` 用的是同一套，不会出现「某处看得见
/// 某处看不见」。
pub(crate) async fn load_all_skills() -> Vec<crate::agent::skills::loader::SkillDefinition> {
    use crate::agent::skills::loader::SkillLoader;
    use crate::core::config::storage::Storage;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let storage = Storage::new(cwd);
    let mut all = Vec::new();
    for dir in storage.skill_roots() {
        all.extend(SkillLoader::load_skills_from_dir(&dir).await);
    }
    all
}

/// `/skill-search <query>` — 在已装载技能里做意图归一 + 相关性排序
pub async fn skill_search(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    use crate::core::skill_search::{
        intent_normalize::IntentNormalizer, local_search::LocalSkillSearch,
        local_search::SkillInfo, SkillSearchConfig,
    };

    let query = args.join(" ");
    let skills = load_all_skills().await;

    if query.is_empty() {
        let mut out = format!("## Skill search\n\n{} skill(s) indexed.\n\n", skills.len());
        out.push_str("Usage: `/skill-search <what you want to do>`\n");
        if !skills.is_empty() {
            out.push_str("\nIndexed:\n");
            for s in skills.iter().take(20) {
                out.push_str(&format!("- `{}`\n", s.name));
            }
        }
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let normalizer = IntentNormalizer::new();
    let intent = normalizer.normalize(&query);
    let infos: Vec<SkillInfo> = skills
        .iter()
        .map(|s| SkillInfo {
            id: s.name.clone(),
            name: s.name.clone(),
            description: if s.description.is_empty() {
                s.metadata.when_to_use.clone()
            } else {
                Some(s.description.clone())
            },
        })
        .collect();

    let searcher = LocalSkillSearch::new(SkillSearchConfig::from_env());
    let keyword_query = intent.keywords.join(" ");
    let hits = if keyword_query.is_empty() {
        Vec::new()
    } else {
        searcher.search(&keyword_query, &infos)
    };
    let results = if hits.is_empty() {
        searcher.search(&query, &infos)
    } else {
        hits
    };

    let mut out = format!(
        "## Skill search: `{}`\n\n- intent: `{:?}` ({:.0}% confidence)\n- keywords: {}\n\n",
        query,
        intent.intent_type,
        intent.confidence * 100.0,
        if intent.keywords.is_empty() {
            "-".to_string()
        } else {
            intent.keywords.join(", ")
        }
    );
    if results.is_empty() {
        out.push_str(&format!(
            "No skill scored above the relevance floor across {} indexed skill(s).\n\n\
             Try `/skills list`, or `/skill-store` to install more.\n",
            skills.len()
        ));
    } else {
        for r in &results {
            out.push_str(&format!(
                "- **{}** — {:.0}% ({})\n",
                r.name,
                r.relevance_score * 100.0,
                r.match_reason
            ));
            if let Some(d) = &r.description {
                out.push_str(&format!(
                    "  - {}\n",
                    d.chars().take(120).collect::<String>()
                ));
            }
        }
        out.push_str("\nRun one with `/skill <name>`.\n");
    }
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/skill-learning` —— 技能学习管线状态与本能探测。
///
/// 学习管线（core/skill_learning）目前是进程内的：观察结果不落盘，
/// 也还没有接到会话循环上，所以这里如实报告策略与真实持久化的技能来源，
/// 并提供 `probe` 子命令对真实文本跑一次 InstinctParser。
pub async fn skill_learning(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    use crate::core::skill_learning::{InstinctParser, LearningPolicy, SkillLearningManager};

    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    if sub == "probe" {
        let text = args[1..].join(" ");
        if text.trim().is_empty() {
            push_msg(&mut ctx, "Usage: `/skill-learning probe <text>`");
            return Ok(());
        }
        let parser = InstinctParser::new();
        let out = match parser.parse(&text) {
            Some(i) => format!(
                "## Instinct match\n\n- input: `{}`\n- instinct: **{}**\n- trigger: `{}`\n- response: {}\n- priority: {}\n",
                text, i.name, i.trigger, i.response, i.priority
            ),
            None => format!(
                "## Instinct match\n\nNo built-in instinct matched `{}`.\n\n\
                 Built-in patterns: `error_fix` (error/fix/bug), `refactor` (refactor/improve/optimize), \
                 `test` (test/verify/check).\n",
                text
            ),
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let mgr = SkillLearningManager::from_env();
    let policy = LearningPolicy::from_env();
    let skills = load_all_skills().await;
    let learned = mgr.get_all_skills().len();

    let mut out = String::from("## Skill learning\n\n");
    out.push_str(&format!(
        "- pipeline: {} (`STAR_SKILL_LEARNING_ENABLED`)\n\
         - auto learning: {} (`STAR_SKILL_AUTO_LEARNING`)\n\
         - evolution: {} (`STAR_SKILL_EVOLUTION`)\n\n",
        on_off(mgr.is_enabled()),
        on_off(policy.auto_learning_enabled),
        on_off(policy.evolution_enabled),
    ));
    out.push_str("### Policy\n\n");
    out.push_str(&format!(
        "| setting | value | env |\n|---|---|---|\n\
         | min observations | {} | `STAR_SKILL_MIN_OBSERVATIONS` |\n\
         | min success rate | {:.0}% | `STAR_SKILL_MIN_SUCCESS_RATE` |\n\
         | max skills | {} | `STAR_SKILL_MAX_COUNT` |\n\
         | expiry | {} days | `STAR_SKILL_EXPIRY_DAYS` |\n\n",
        policy.min_observations,
        policy.min_success_rate * 100.0,
        policy.max_skills,
        policy.skill_expiry_secs / 86_400,
    ));
    out.push_str(&format!(
        "### State\n\n- learned skills this process: **{}**\n\
         - authored skills on disk: **{}** (`.star/skills`, `~/.star/skills`)\n\n",
        learned,
        skills.len()
    ));
    out.push_str(
        "The observer/generator/evolution pipeline is in-process only — observations are not\n\
         persisted and nothing in the session loop feeds it yet, so the learned count resets\n\
         every launch. Durable skills are the `SKILL.md` files above; author them by hand or\n\
         install them with `/skill-store install <name>`.\n\n\
         Subcommand: `probe <text>` — run the instinct parser against real text.\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/skill-store` —— 技能/扩展商店：浏览、搜索、安装、卸载。
///
/// 接 `core::extensions::marketplace`（真实写入 `~/.star/extensions/`
/// 与 `registry.json`）。
pub async fn skill_store(mut ctx: CommandContext<'_>, args: &[String]) -> CommandResult {
    use crate::core::extensions::marketplace::Marketplace;
    use crate::core::extensions::registry::ExtensionRegistryManager;
    use crate::core::extensions::types::ExtensionType;

    let market = Marketplace::new();
    let registry = ExtensionRegistryManager::new();
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");

    match sub {
        "install" | "add" => {
            let name = match args.get(1) {
                Some(n) => n.clone(),
                None => {
                    push_msg(&mut ctx, "Usage: `/skill-store install <name>`");
                    return Ok(());
                }
            };
            match market.install(&name).await {
                Ok(r) => push_msg(
                    &mut ctx,
                    format!(
                        "{} **{}** ({}) — {}\n\nReload with `/reload-plugins`.",
                        if r.success { "✓" } else { "✗" },
                        r.name,
                        r.extension_type,
                        r.message
                    ),
                ),
                Err(e) => push_msg(&mut ctx, format!("Install failed: {}", e)),
            }
        }
        "uninstall" | "rm" | "remove" => {
            let name = match args.get(1) {
                Some(n) => n.clone(),
                None => {
                    push_msg(&mut ctx, "Usage: `/skill-store uninstall <name>`");
                    return Ok(());
                }
            };
            match market.uninstall(&name) {
                Ok(r) => push_msg(
                    &mut ctx,
                    format!(
                        "{} **{}** — {}",
                        if r.success { "✓" } else { "✗" },
                        r.name,
                        r.message
                    ),
                ),
                Err(e) => push_msg(&mut ctx, format!("Uninstall failed: {}", e)),
            }
        }
        "search" | "find" => {
            let q = args[1..].join(" ");
            if q.trim().is_empty() {
                push_msg(&mut ctx, "Usage: `/skill-store search <query>`");
                return Ok(());
            }
            let hits = market.search(&q);
            push_msg(
                &mut ctx,
                render_store_entries(&format!("Search: `{}`", q), &hits, &registry),
            );
        }
        "skills" => {
            let hits = market.list_by_type(&ExtensionType::Skill);
            push_msg(&mut ctx, render_store_entries("Skills", &hits, &registry));
        }
        "mcp" => {
            let hits = market.list_by_type(&ExtensionType::Mcp);
            push_msg(
                &mut ctx,
                render_store_entries("MCP servers", &hits, &registry),
            );
        }
        "installed" => {
            let entries = registry.list_all();
            let mut out = String::from("## Installed extensions\n\n");
            if entries.is_empty() {
                out.push_str("None yet. `/skill-store` to browse.\n");
            } else {
                out.push_str("| name | type | version | enabled |\n|---|---|---|---|\n");
                for e in &entries {
                    out.push_str(&format!(
                        "| `{}` | {} | {} | {} |\n",
                        e.name,
                        e.extension_type,
                        e.version,
                        on_off(e.enabled)
                    ));
                }
                out.push_str(&format!(
                    "\nRoot: `{}`\n",
                    ExtensionRegistryManager::global_extensions_dir().display()
                ));
            }
            push_msg(&mut ctx, out);
        }
        "all" => {
            let all = market.list_all();
            push_msg(
                &mut ctx,
                render_store_entries("All entries", &all, &registry),
            );
        }
        _ => {
            let featured = market.list_featured();
            let mut out = render_store_entries("Featured", &featured, &registry);
            out.push_str(
                "\nSubcommands: `all`, `skills`, `mcp`, `search <q>`, `installed`, \
                 `install <name>`, `uninstall <name>`\n",
            );
            push_msg(&mut ctx, out);
        }
    }
    Ok(())
}

/// 渲染商店条目表格，并标注是否已安装。
pub(crate) fn render_store_entries(
    title: &str,
    entries: &[crate::core::extensions::types::MarketplaceEntry],
    registry: &crate::core::extensions::registry::ExtensionRegistryManager,
) -> String {
    let mut out = format!("## Skill store — {}\n\n", title);
    if entries.is_empty() {
        out.push_str("No matching entries.\n");
        return out;
    }
    out.push_str("| name | type | version | installed | description |\n|---|---|---|---|---|\n");
    for e in entries.iter().take(40) {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            e.name,
            e.extension_type,
            e.version,
            if registry.is_installed(&e.name) {
                "✓"
            } else {
                "-"
            },
            e.description.chars().take(70).collect::<String>()
        ));
    }
    if entries.len() > 40 {
        out.push_str(&format!("\n… {} more\n", entries.len() - 40));
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Deep planning / review
// ═══════════════════════════════════════════════════════════════════════════
