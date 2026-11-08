//! parity 命令族 —— memory（对标 Claude Code 命令面，拆自 parity.rs 2026-11-09）

use crate::commands::execution::{CommandContext, CommandResult};
use crate::commands::parity::core::is_secretish;
use crate::commands::parity::core::on_off;
use crate::commands::parity::core::push_msg;
use crate::commands::parity::core::star_dir;
use crate::commands::parity::team_mail::now_ms;
use crate::runtime::messages::AgentRequest;
use crate::types::ChatEntry;
use std::path::PathBuf;

/// 项目级分类记忆库根目录：`.star/memory/`
pub(crate) fn classified_dir() -> Result<PathBuf, String> {
    Ok(star_dir()?.join("memory"))
}

pub(crate) fn classified_manager(
) -> Result<crate::core::memory::classification::ClassifiedMemoryManager, String> {
    use crate::core::memory::classification::ClassifiedMemoryManager;
    let mut mgr = ClassifiedMemoryManager::new(classified_dir()?);
    mgr.initialize()?;
    let _ = mgr.load_index();
    Ok(mgr)
}

/// `/local-memory` — 项目级四类型记忆库（user/feedback/project/reference）
pub async fn local_memory(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mut mgr = match classified_manager() {
        Ok(m) => m,
        Err(e) => {
            push_msg(&mut ctx, format!("⚠️ {}", e));
            return Ok(());
        }
    };

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let out = match sub.as_str() {
        "" | "list" | "stats" => render_local_memory(&mgr),
        "search" | "recall" => {
            let q = args[1..].join(" ");
            if q.is_empty() {
                "Usage: `/local-memory search <query>`".to_string()
            } else {
                let hits = mgr.recall(&q, 10);
                if hits.is_empty() {
                    format!("No memories matching `{}`.", q)
                } else {
                    let mut s = format!("## Recall: `{}`\n\n", q);
                    for h in hits {
                        s.push_str(&format!(
                            "- [{}] **{}** — `{}`\n",
                            h.memory_type, h.title, h.file_path
                        ));
                    }
                    s
                }
            }
        }
        "add" | "remember" => add_local_memory(&mut mgr, &args[1..]),
        _ => "Usage: `/local-memory` | `/local-memory add <type> <title> :: <content>` | \
              `/local-memory search <query>`\n\nTypes: user · feedback · project · reference"
            .to_string(),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/local-memory add <type> <title> :: <content>`
pub(crate) fn add_local_memory(
    mgr: &mut crate::core::memory::classification::ClassifiedMemoryManager,
    rest: &[String],
) -> String {
    use crate::core::memory::classification::{ClassifiedMemory, MemoryType};

    if rest.is_empty() {
        return "Usage: `/local-memory add <user|feedback|project|reference> <title> :: <content>`"
            .to_string();
    }
    let Some(mem_type) = MemoryType::from_str(&rest[0]) else {
        return format!(
            "Unknown memory type `{}` — use user, feedback, project or reference.",
            rest[0]
        );
    };
    let body = rest[1..].join(" ");
    let (title, content) = match body.split_once("::") {
        Some((t, c)) => (t.trim().to_string(), c.trim().to_string()),
        None => {
            let t: String = body.chars().take(60).collect();
            (t, body.clone())
        }
    };
    if content.is_empty() {
        return "Nothing to remember — give a title and content.".to_string();
    }

    let now = now_ms() as u64 / 1000;
    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let memory = ClassifiedMemory {
        id: id.clone(),
        memory_type: mem_type.clone(),
        title: title.clone(),
        content,
        tags: Vec::new(),
        relevance_score: 0.5,
        created_at: now,
        accessed_at: now,
        access_count: 0,
        source: Some("slash:/local-memory".to_string()),
    };
    if !mgr.validate_memory(&memory) {
        return "Rejected: memory failed validation (empty or looks like a secret).".to_string();
    }
    match mgr.store(memory) {
        Ok(()) => {
            let _ = mgr.save_index();
            format!("🧠 Stored [{}] **{}** (`{}`).", mem_type, title, id)
        }
        Err(e) => format!("⚠️ {}", e),
    }
}

pub(crate) fn render_local_memory(
    mgr: &crate::core::memory::classification::ClassifiedMemoryManager,
) -> String {
    use crate::core::memory::classification::MemoryType;
    let dir = classified_dir().unwrap_or_default();
    let mut out = format!("## Local memory\n\n`{}`\n\n", dir.display());

    let stats = mgr.stats();
    let mut total = 0usize;
    for t in [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ] {
        let n = stats.get(&t.to_string()).copied().unwrap_or(0);
        total += n;
        out.push_str(&format!(
            "- **{}** — {} entr{}\n",
            t,
            n,
            if n == 1 { "y" } else { "ies" }
        ));
    }

    if total == 0 {
        out.push_str("\n_empty_ — add one with `/local-memory add project <title> :: <content>`\n");
        return out;
    }

    out.push_str("\n### Recent\n\n");
    for t in [
        MemoryType::User,
        MemoryType::Feedback,
        MemoryType::Project,
        MemoryType::Reference,
    ] {
        for e in mgr.by_type(&t).into_iter().take(3) {
            out.push_str(&format!("- [{}] **{}** — `{}`\n", t, e.title, e.id));
        }
    }
    out.push_str("\n`/local-memory search <query>` · `/memory-stores` for every store\n");
    out
}

/// 统计目录里的文件数与总字节
pub(crate) fn dir_stats(dir: &std::path::Path) -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => {
                    files += 1;
                    bytes += m.len();
                }
                Err(_) => {}
            }
        }
    }
    (files, bytes)
}

pub(crate) fn human_bytes(b: u64) -> String {
    if b < 1024 {
        format!("{} B", b)
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    }
}

/// 一行 store 描述
pub(crate) fn store_row(label: &str, path: &std::path::Path, is_dir: bool) -> String {
    if !path.exists() {
        return format!("| {} | `{}` | — | absent |\n", label, path.display());
    }
    if is_dir {
        let (n, b) = dir_stats(path);
        format!(
            "| {} | `{}` | {} file(s) / {} | present |\n",
            label,
            path.display(),
            n,
            human_bytes(b)
        )
    } else {
        let b = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        format!(
            "| {} | `{}` | {} | present |\n",
            label,
            path.display(),
            human_bytes(b)
        )
    }
}

/// `/memory-stores` — 列出本会话可见的所有记忆/上下文存储
pub async fn memory_stores(mut ctx: CommandContext<'_>, _args: Vec<String>) -> CommandResult {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let team = crate::core::team_memory::TeamMemoryConfig::from_env();

    let mut out =
        String::from("## Memory stores\n\n| store | path | size | state |\n|---|---|---|---|\n");
    out.push_str(&store_row(
        "project memory (/memory)",
        &cwd.join(".star").join("memory.md"),
        false,
    ));
    out.push_str(&store_row(
        "classified (/local-memory)",
        &cwd.join(".star").join("memory"),
        true,
    ));
    for name in ["STAR.md", "STARCODE.md", "CLAUDE.md", "AGENTS.md"] {
        out.push_str(&store_row(name, &cwd.join(name), false));
    }
    out.push_str(&store_row(
        "global CLAUDE.md",
        &home.join(".claude").join("CLAUDE.md"),
        false,
    ));
    out.push_str(&store_row(
        "user memdir",
        &home.join(".starcode").join("memory"),
        true,
    ));
    out.push_str(&store_row(
        "session transcripts",
        &home.join(".star").join("transcripts"),
        true,
    ));

    out.push_str("\n### Shared / team memory\n\n");
    out.push_str(&format!(
        "- enabled: `{}` (`STAR_TEAM_MEMORY_ENABLED`)\n- team id: `{}`\n\
         - sync endpoint: `{}`\n- secret scanning: `{}`\n",
        on_off(team.enabled),
        team.team_id.as_deref().unwrap_or("-"),
        team.sync_endpoint.as_deref().unwrap_or("-"),
        on_off(team.secret_scanning)
    ));
    if let Some(id) = &team.team_id {
        out.push_str(&store_row(
            "team memdir",
            &home.join(".starcode").join("memory").join("teams").join(id),
            true,
        ));
    }
    out.push_str(
        "\n`/memory show` edits the project file · `/local-memory` manages the classified store\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

/// 本地凭据库（keychain 优先，Linux 上回退到 `~/.star/secure_storage.json`）
pub(crate) fn vault_manager() -> crate::core::secure_storage::SecureStorageManager {
    crate::core::secure_storage::SecureStorageManager::from_env()
}

pub(crate) fn vault_path() -> String {
    std::env::var("STAR_SECURE_STORAGE_PATH").unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".star")
            .join("secure_storage.json")
            .display()
            .to_string()
    })
}

/// `/local-vault` — 本机凭据库：list / set / get / rm
pub async fn local_vault(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    let mgr = vault_manager();
    let backend = mgr.backend_type();
    let plaintext = matches!(
        backend,
        crate::core::secure_storage::StorageBackend::PlainText
            | crate::core::secure_storage::StorageBackend::Auto
    ) && !cfg!(target_os = "macos");

    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let out = match sub.as_str() {
        "" | "list" | "ls" => {
            let keys = mgr.list().unwrap_or_default();
            let mut s = format!(
                "## Local vault\n\n- backend: `{:?}`{}\n- store: `{}`\n- entries: `{}`\n\n",
                backend,
                if plaintext {
                    " (plaintext fallback)"
                } else {
                    ""
                },
                vault_path(),
                keys.len()
            );
            if keys.is_empty() {
                s.push_str("_empty_ — add one with `/local-vault set <key> <value>`\n");
            } else {
                let mut sorted = keys;
                sorted.sort();
                for k in sorted {
                    s.push_str(&format!("- `{}`\n", k));
                }
            }
            if plaintext {
                s.push_str(
                    "\n⚠️ Values are stored unencrypted on this platform. Prefer env vars or a \
                     system keychain for production secrets.\n",
                );
            }
            s
        }
        "set" | "put" => match (args.get(1), args.get(2)) {
            (Some(k), Some(v)) => match mgr.store(k, v) {
                Ok(()) => format!("🔐 Stored `{}` in the local vault.", k),
                Err(e) => format!("⚠️ {}", e),
            },
            _ => "Usage: `/local-vault set <key> <value>`".to_string(),
        },
        "get" => match args.get(1) {
            None => "Usage: `/local-vault get <key>`".to_string(),
            Some(k) => match mgr.get(k) {
                // Vault 条目一律按机密处理，只回报长度而不回显明文
                Ok(Some(v)) => format!("`{}` = <set, {} chars>", k, v.chars().count()),
                Ok(None) => format!("`{}` is not in the vault.", k),
                Err(e) => format!("⚠️ {}", e),
            },
        },
        "rm" | "delete" => match args.get(1) {
            None => "Usage: `/local-vault rm <key>`".to_string(),
            Some(k) => match mgr.delete(k) {
                Ok(()) => format!("🗑 Removed `{}`.", k),
                Err(e) => format!("⚠️ {}", e),
            },
        },
        _ => "Usage: `/local-vault [list|set <k> <v>|get <k>|rm <k>]`".to_string(),
    };
    push_msg(&mut ctx, out);
    Ok(())
}

/// `/vault` — 远端凭据/设置来源总览；`/vault sync` 拉一次远端设置
pub async fn vault(mut ctx: CommandContext<'_>, args: Vec<String>) -> CommandResult {
    if args.first().map(|s| s.to_lowercase()).as_deref() == Some("sync") {
        let endpoint = ctx.state.remote_settings.endpoint.clone();
        let out = match endpoint {
            None => "No remote settings endpoint configured. Set one with `/vault endpoint <url>`."
                .to_string(),
            Some(url) => match ctx.state.remote_settings.sync().await {
                Ok(()) => format!("🔄 Synced remote settings from `{}`.", url),
                Err(e) => format!("⚠️ {}", e),
            },
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    if args.first().map(|s| s.to_lowercase()).as_deref() == Some("endpoint") {
        let out = match args.get(1) {
            None => "Usage: `/vault endpoint <url>`".to_string(),
            Some(url) => {
                ctx.state.remote_settings.set_endpoint(url);
                format!(
                    "✅ Remote settings endpoint set to `{}` (this session).",
                    url
                )
            }
        };
        push_msg(&mut ctx, out);
        return Ok(());
    }

    let store = crate::core::config::provider_store::ProviderStore::new();
    let configured = store.configured_provider_ids().await.unwrap_or_default();
    let env_keys: Vec<String> = std::env::vars()
        .map(|(k, _)| k)
        .filter(|k| is_secretish(k) && (k.starts_with("STAR_") || k.contains("API")))
        .collect();

    let (endpoint, last_sync, interval) = {
        let rs = &ctx.state.remote_settings;
        (rs.endpoint.clone(), rs.last_sync, rs.sync_interval_secs)
    };

    let mut out = String::from("## Vaults\n\n### Remote settings channel\n\n");
    out.push_str(&format!(
        "- endpoint: `{}`\n- last sync: `{}`\n- interval: `{}s`\n",
        endpoint.as_deref().unwrap_or("-"),
        last_sync
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "never".to_string()),
        interval
    ));

    out.push_str("\n### Credential sources\n\n");
    out.push_str(&format!(
        "- provider store: `{}` provider(s) configured — `/provider`\n",
        configured.len()
    ));
    out.push_str(&format!(
        "- local vault: `{}` — `/local-vault`\n",
        vault_path()
    ));
    if env_keys.is_empty() {
        out.push_str("- environment: no secret-looking vars set\n");
    } else {
        let mut sorted = env_keys;
        sorted.sort();
        out.push_str(&format!(
            "- environment: {} secret-looking var(s) — {}\n",
            sorted.len(),
            sorted
                .iter()
                .take(8)
                .map(|k| format!("`{}`", k))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(
        "\nNo hosted vault provider is bundled. Point `STAR_SECURE_STORAGE_PATH` at a mounted \
         secret volume, or feed credentials through the environment.\n\n\
         `/vault endpoint <url>` · `/vault sync`\n",
    );
    push_msg(&mut ctx, out);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 技能：搜索 / 学习 / 市场
// ═══════════════════════════════════════════════════════════════════════════
